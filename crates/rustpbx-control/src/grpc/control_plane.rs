use crate::{
    grpc::proto::control::{
        AclRuleList, CallRecordReport, ConfigChangeEvent, GetAclRulesRequest,
        GetRouteRulesRequest, GetTrunkConfigsRequest, GetWorkersRequest, HeartbeatRequest,
        HeartbeatResponse, RegisterAck, ReportAck, RouteRuleList, TrunkConfigList, WatchRequest,
        WorkerInfo, WorkerList,
        control_plane_server::ControlPlane,
    },
    store::Store,
    worker_registry::WorkerRegistry,
};
use chrono::Utc;
use futures::Stream;
use std::{pin::Pin, sync::Arc};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

pub struct ControlPlaneService {
    pub store: Arc<Store>,
    pub workers: Arc<WorkerRegistry>,
    pub change_tx: broadcast::Sender<ConfigChangeEvent>,
}

impl ControlPlaneService {
    pub fn new(store: Arc<Store>, workers: Arc<WorkerRegistry>) -> Self {
        let (change_tx, _) = broadcast::channel(256);
        Self { store, workers, change_tx }
    }

    /// Broadcast a config-change event to all streaming watchers.
    /// Unused for now (events are pushed from the mutating handlers directly);
    /// retained for future administrative triggers.
    #[allow(dead_code)]
    pub fn broadcast(&self, event: ConfigChangeEvent) {
        let _ = self.change_tx.send(event);
    }
}

type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;

#[tonic::async_trait]
impl ControlPlane for ControlPlaneService {
    // ── Configuration pull ────────────────────────────────────────────────────

    async fn get_trunk_configs(
        &self,
        request: Request<GetTrunkConfigsRequest>,
    ) -> Result<Response<TrunkConfigList>, Status> {
        let req = request.into_inner();
        info!(edge_id = ?req.edge_id, tenant_id = ?req.tenant_id, "get_trunk_configs");

        let trunks = self
            .store
            .load_trunks(req.tenant_id)
            .await
            .map_err(|e| {
                warn!(error = %e, "db error loading trunks");
                Status::internal(e.to_string())
            })?;

        info!(count = trunks.len(), "trunk configs sent");
        Ok(Response::new(TrunkConfigList {
            trunks,
            version: version_now(),
        }))
    }

    async fn get_route_rules(
        &self,
        request: Request<GetRouteRulesRequest>,
    ) -> Result<Response<RouteRuleList>, Status> {
        let req = request.into_inner();
        info!(tenant_id = ?req.tenant_id, "get_route_rules");

        let rules = self
            .store
            .load_routes(req.tenant_id)
            .await
            .map_err(|e| {
                warn!(error = %e, "db error loading routes");
                Status::internal(e.to_string())
            })?;

        info!(count = rules.len(), "route rules sent");
        Ok(Response::new(RouteRuleList {
            rules,
            version: version_now(),
        }))
    }

    async fn get_acl_rules(
        &self,
        request: Request<GetAclRulesRequest>,
    ) -> Result<Response<AclRuleList>, Status> {
        let req = request.into_inner();
        info!(tenant_id = ?req.tenant_id, "get_acl_rules");

        // ACL rules currently come from config files in the main rustpbx instance.
        // Control Plane returns an empty list for now; future: read from a
        // dedicated rustpbx_acl_rules table managed by the Control Plane.
        let _ = req.tenant_id;
        Ok(Response::new(AclRuleList { rules: vec![], version: version_now() }))
    }

    // ── Config push (server streaming) ────────────────────────────────────────

    type WatchConfigChangesStream = BoxStream<ConfigChangeEvent>;

    async fn watch_config_changes(
        &self,
        request: Request<WatchRequest>,
    ) -> Result<Response<Self::WatchConfigChangesStream>, Status> {
        let req = request.into_inner();
        info!(edge_id = ?req.edge_id, worker_id = ?req.worker_id, "watch_config_changes subscribed");

        let rx = self.change_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(event) => Some(Ok(event)),
            Err(_) => None, // lagged receiver — drop and continue
        });

        Ok(Response::new(Box::pin(stream)))
    }

    // ── CDR ───────────────────────────────────────────────────────────────────

    async fn report_call_record(
        &self,
        request: Request<CallRecordReport>,
    ) -> Result<Response<ReportAck>, Status> {
        let rec = request.into_inner();
        info!(
            call_id  = %rec.call_id,
            tenant   = ?rec.tenant_id,
            caller   = %rec.caller,
            callee   = %rec.callee,
            status   = %rec.status,
            duration = rec.duration_secs,
            worker   = ?rec.worker_id,
            "cdr received"
        );

        // Persist via store (raw INSERT into rustpbx_call_records)
        if let Err(e) = self.store.persist_cdr(&rec).await {
            warn!(error = %e, call_id = %rec.call_id, "cdr persist failed");
            // Still ack — losing a CDR is better than blocking the worker
        }

        Ok(Response::new(ReportAck { accepted: true }))
    }

    // ── Worker lifecycle ──────────────────────────────────────────────────────

    async fn register_worker(
        &self,
        request: Request<WorkerInfo>,
    ) -> Result<Response<RegisterAck>, Status> {
        use crate::worker_registry::WorkerEntry;
        let info = request.into_inner();
        info!(worker_id = %info.worker_id, sip_addr = %info.sip_addr, max = info.max_concurrent, "worker register");

        self.workers.register(WorkerEntry {
            worker_id: info.worker_id,
            sip_addr: info.sip_addr,
            rtp_external_ip: info.rtp_external_ip,
            rtp_start_port: info.rtp_start_port,
            rtp_end_port: info.rtp_end_port,
            max_concurrent: info.max_concurrent,
            active_calls: info.active_calls,
            cpu_usage: 0.0,
            registered_at: Utc::now(),
            last_heartbeat: tokio::time::Instant::now(),
            draining: false,
        });

        Ok(Response::new(RegisterAck { accepted: true }))
    }

    async fn worker_heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let hb = request.into_inner();
        let known = self.workers.heartbeat(
            &hb.worker_id,
            hb.active_calls,
            hb.cpu_usage,
            hb.rtp_ports_used,
        );
        Ok(Response::new(HeartbeatResponse { drain: !known }))
    }

    // ── Worker discovery ──────────────────────────────────────────────────────

    async fn get_available_workers(
        &self,
        request: Request<GetWorkersRequest>,
    ) -> Result<Response<WorkerList>, Status> {
        let req = request.into_inner();
        let _ = req.tenant_id;

        let workers = self
            .workers
            .available()
            .into_iter()
            .map(|w| WorkerInfo {
                worker_id: w.worker_id,
                sip_addr: w.sip_addr,
                rtp_external_ip: w.rtp_external_ip,
                rtp_start_port: w.rtp_start_port,
                rtp_end_port: w.rtp_end_port,
                max_concurrent: w.max_concurrent,
                active_calls: w.active_calls,
                labels: Default::default(),
            })
            .collect();

        Ok(Response::new(WorkerList { workers }))
    }
}

/// Use current unix seconds as a cheap monotonic version number.
fn version_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
