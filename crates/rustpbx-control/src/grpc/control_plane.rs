use crate::{
    grpc::proto::control::{
        AclRuleList, AcquireSlotRequest, AcquireSlotResponse, CallRecordReport, ConfigChangeEvent,
        EdgeHeartbeatRequest, EdgeInfo, ExtensionLocationReport, GetAclRulesRequest,
        GetRouteRulesRequest, GetTrunkConfigsRequest, GetWorkersRequest, HeartbeatRequest,
        HeartbeatResponse, RegisterAck, ReportAck, RouteRuleList, TrunkConfigList, WatchRequest,
        WorkerInfo, WorkerList, control_plane_server::ControlPlane,
    },
    raft::registry::RaftRegistry,
    settings::PlatformSettings,
    store::Store,
};
use futures::Stream;
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

pub struct ControlPlaneService {
    pub store: Arc<Store>,
    pub workers: RaftRegistry,
    pub change_tx: broadcast::Sender<ConfigChangeEvent>,
}

impl ControlPlaneService {
    #[cfg(test)]
    pub fn new(store: Arc<Store>, workers: RaftRegistry) -> Self {
        let (change_tx, _) = broadcast::channel(256);
        Self {
            store,
            workers,
            change_tx,
        }
    }

    pub fn with_change_tx(
        store: Arc<Store>,
        workers: RaftRegistry,
        change_tx: broadcast::Sender<ConfigChangeEvent>,
    ) -> Self {
        Self {
            store,
            workers,
            change_tx,
        }
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

        let trunks = self.store.load_trunks(req.tenant_id).await.map_err(|e| {
            warn!(error = %e, "db error loading trunks");
            Status::internal(e.to_string())
        })?;

        info!(count = trunks.len(), "trunk configs sent");
        Ok(Response::new(TrunkConfigList {
            trunks,
            version: PlatformSettings::new(&self.store.db).config_version().await,
        }))
    }

    async fn get_route_rules(
        &self,
        request: Request<GetRouteRulesRequest>,
    ) -> Result<Response<RouteRuleList>, Status> {
        let req = request.into_inner();
        info!(tenant_id = ?req.tenant_id, "get_route_rules");

        let rules = self.store.load_routes(req.tenant_id).await.map_err(|e| {
            warn!(error = %e, "db error loading routes");
            Status::internal(e.to_string())
        })?;

        info!(count = rules.len(), "route rules sent");
        Ok(Response::new(RouteRuleList {
            rules,
            version: PlatformSettings::new(&self.store.db).config_version().await,
        }))
    }

    async fn get_acl_rules(
        &self,
        request: Request<GetAclRulesRequest>,
    ) -> Result<Response<AclRuleList>, Status> {
        let req = request.into_inner();
        info!(tenant_id = ?req.tenant_id, "get_acl_rules");

        let rules = self
            .store
            .load_acl_rules(req.tenant_id)
            .await
            .map_err(|e| Status::internal(format!("load acl rules: {e}")))?;

        Ok(Response::new(AclRuleList {
            rules,
            version: PlatformSettings::new(&self.store.db).config_version().await,
        }))
    }

    async fn get_queues(
        &self,
        request: Request<crate::grpc::proto::control::GetQueuesRequest>,
    ) -> Result<Response<crate::grpc::proto::control::QueueConfigList>, Status> {
        let req = request.into_inner();
        info!(tenant_id = ?req.tenant_id, "get_queues");
        let pairs = self
            .store
            .load_queues(req.tenant_id)
            .await
            .map_err(|e| Status::internal(format!("load queues: {e}")))?;
        let queues = pairs
            .into_iter()
            .map(|(name, spec_json)| crate::grpc::proto::control::QueueConfig { name, spec_json })
            .collect();
        Ok(Response::new(
            crate::grpc::proto::control::QueueConfigList { queues },
        ))
    }

    async fn get_ivrs(
        &self,
        request: Request<crate::grpc::proto::control::GetIvrsRequest>,
    ) -> Result<Response<crate::grpc::proto::control::IvrConfigList>, Status> {
        let req = request.into_inner();
        info!(tenant_id = ?req.tenant_id, "get_ivrs");
        let pairs = self
            .store
            .load_ivrs(req.tenant_id)
            .await
            .map_err(|e| Status::internal(format!("load ivrs: {e}")))?;
        let ivrs = pairs
            .into_iter()
            .map(|(name, spec_json)| crate::grpc::proto::control::IvrConfig { name, spec_json })
            .collect();
        Ok(Response::new(crate::grpc::proto::control::IvrConfigList {
            ivrs,
        }))
    }

    // ── Config push (server streaming) ────────────────────────────────────────

    type WatchConfigChangesStream = BoxStream<ConfigChangeEvent>;

    async fn watch_config_changes(
        &self,
        request: Request<WatchRequest>,
    ) -> Result<Response<Self::WatchConfigChangesStream>, Status> {
        let req = request.into_inner();
        info!(
            edge_id = ?req.edge_id,
            worker_id = ?req.worker_id,
            from_version = req.from_version,
            "watch_config_changes subscribed"
        );

        let current_version = PlatformSettings::new(&self.store.db).config_version().await;
        let initial =
            (req.from_version < current_version).then(|| ConfigChangeEvent {
                change_type:
                    crate::grpc::proto::control::config_change_event::ChangeType::PlatformChanged
                        as i32,
                name: Some("resync".to_string()),
                trunk: None,
                version: current_version,
            });
        let rx = self.change_tx.subscribe();
        let live = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(event) => Some(Ok(event)),
            Err(_) => None, // lagged receiver — drop and continue
        });
        let stream = tokio_stream::iter(initial).map(Ok).chain(live);

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

        // Release the per-tenant concurrency slot reserved at call setup. A
        // no-op if none was held (e.g. outbound, or slots disabled). Best-effort.
        if let Err(e) = self.workers.release_call_slot(&rec.call_id).await {
            warn!(error = %e, call_id = %rec.call_id, "call-slot release failed");
        }

        Ok(Response::new(ReportAck { accepted: true }))
    }

    // ── Worker lifecycle ──────────────────────────────────────────────────────

    async fn register_worker(
        &self,
        request: Request<WorkerInfo>,
    ) -> Result<Response<RegisterAck>, Status> {
        use crate::raft::types::WorkerRecord;
        let info = request.into_inner();
        info!(worker_id = %info.worker_id, sip_addr = %info.sip_addr, max = info.max_concurrent, "worker register");

        self.workers
            .register(WorkerRecord {
                worker_id: info.worker_id,
                sip_addr: info.sip_addr,
                rtp_external_ip: info.rtp_external_ip,
                rtp_start_port: info.rtp_start_port,
                rtp_end_port: info.rtp_end_port,
                max_concurrent: info.max_concurrent,
                active_calls: info.active_calls,
                cpu_usage: 0.0,
                labels: info.labels,
                capabilities: info.capabilities,
                edge_worker_addr: info.edge_worker_addr,
                nat_type: info.nat_type,
                // Timestamps are stamped by the registry at propose time.
                registered_at_ms: 0,
                last_heartbeat_ms: 0,
                draining: false,
            })
            .await
            .map_err(|e| Status::internal(format!("raft write failed: {e}")))?;

        Ok(Response::new(RegisterAck { accepted: true }))
    }

    async fn worker_heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let hb = request.into_inner();
        let _ = hb.rtp_ports_used; // reserved for future metrics
        let known = self
            .workers
            .heartbeat(&hb.worker_id, hb.active_calls, hb.cpu_usage)
            .await
            .map_err(|e| Status::internal(format!("raft write failed: {e}")))?;
        Ok(Response::new(HeartbeatResponse { drain: !known }))
    }

    async fn report_extension_location(
        &self,
        request: Request<ExtensionLocationReport>,
    ) -> Result<Response<ReportAck>, Status> {
        let report = request.into_inner();
        let extension = report.extension.trim();
        if extension.is_empty() || report.worker_id.trim().is_empty() {
            return Err(Status::invalid_argument(
                "extension and worker_id are required",
            ));
        }

        let affinity_key = extension_affinity_key(report.tenant_id, extension);
        if report.expires_secs == 0 {
            self.workers
                .unbind_affinity_worker(affinity_key.clone(), report.worker_id.clone())
                .await
                .map_err(|e| Status::internal(format!("raft write failed: {e}")))?;
            info!(
                affinity_key = %affinity_key,
                worker_id = %report.worker_id,
                "extension location unbound"
            );
        } else {
            let ttl = Duration::from_secs(report.expires_secs as u64);
            let contact = report.contact.trim();
            if contact.is_empty() {
                self.workers
                    .bind_affinity_ttl(affinity_key.clone(), report.worker_id.clone(), ttl)
                    .await
                    .map_err(|e| Status::internal(format!("raft write failed: {e}")))?;
            } else {
                self.workers
                    .bind_affinity_contact_ttl(
                        affinity_key.clone(),
                        report.worker_id.clone(),
                        contact.to_string(),
                        contact_q_milli(contact),
                        ttl,
                    )
                    .await
                    .map_err(|e| Status::internal(format!("raft write failed: {e}")))?;
            }
            info!(
                affinity_key = %affinity_key,
                worker_id = %report.worker_id,
                contact = %report.contact,
                expires_secs = report.expires_secs,
                "extension location bound"
            );
        }

        Ok(Response::new(ReportAck { accepted: true }))
    }

    // ── Edge lifecycle ────────────────────────────────────────────────────────

    async fn register_edge(
        &self,
        request: Request<EdgeInfo>,
    ) -> Result<Response<RegisterAck>, Status> {
        use crate::raft::types::EdgeRecord;
        let info = request.into_inner();
        info!(edge_id = %info.edge_id, sip_addr = %info.sip_addr, version = %info.version, "edge register");

        self.workers
            .register_edge(EdgeRecord {
                edge_id: info.edge_id,
                public_ip: info.public_ip,
                sip_addr: info.sip_addr,
                transports: info.transports,
                region: info.region,
                version: info.version,
                active_calls: info.active_calls,
                nat_type: info.nat_type,
                registered_at_ms: 0,
                last_heartbeat_ms: 0,
            })
            .await
            .map_err(|e| Status::internal(format!("raft write failed: {e}")))?;

        Ok(Response::new(RegisterAck { accepted: true }))
    }

    async fn edge_heartbeat(
        &self,
        request: Request<EdgeHeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let hb = request.into_inner();
        let known = self
            .workers
            .edge_heartbeat(&hb.edge_id, hb.active_calls)
            .await
            .map_err(|e| Status::internal(format!("raft write failed: {e}")))?;
        // Unknown edge (e.g. control restarted) → tell it to re-register.
        Ok(Response::new(HeartbeatResponse { drain: !known }))
    }

    // ── Worker discovery ──────────────────────────────────────────────────────

    async fn get_available_workers(
        &self,
        request: Request<GetWorkersRequest>,
    ) -> Result<Response<WorkerList>, Status> {
        let req = request.into_inner();

        let contacts_by_worker = match req.affinity_key.as_deref() {
            Some(key) if !key.trim().is_empty() => self.workers.contacts_for_affinity(key).await,
            _ => Default::default(),
        };

        let workers = self
            .workers
            .available_with_constraints(
                req.tenant_id,
                &req.required_labels,
                &req.required_capabilities,
                req.affinity_key.as_deref(),
            )
            .await
            .into_iter()
            .map(|w| WorkerInfo {
                extension_contacts: contacts_by_worker
                    .get(&w.worker_id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| crate::grpc::proto::control::ExtensionContact {
                        contact: c.contact,
                        q_milli: c.q_milli as u32,
                        expires_at_unix_ms: c.expires_at_ms.max(0) as u64,
                    })
                    .collect(),
                worker_id: w.worker_id,
                sip_addr: w.sip_addr,
                rtp_external_ip: w.rtp_external_ip,
                rtp_start_port: w.rtp_start_port,
                rtp_end_port: w.rtp_end_port,
                max_concurrent: w.max_concurrent,
                active_calls: w.active_calls,
                labels: w.labels,
                capabilities: w.capabilities,
                edge_worker_addr: w.edge_worker_addr,
                nat_type: w.nat_type,
            })
            .collect();

        Ok(Response::new(WorkerList { workers }))
    }

    // ── Per-tenant concurrency control ────────────────────────────────────────

    async fn acquire_call_slot(
        &self,
        request: Request<AcquireSlotRequest>,
    ) -> Result<Response<AcquireSlotResponse>, Status> {
        let req = request.into_inner();
        // Read the tenant's limit (None/0 → unlimited). tenant_id ≤ 0 means no
        // tenant scope, so no cap — but we still reserve a slot so the call is
        // counted and released uniformly on CDR.
        let max = if req.tenant_id > 0 {
            self.store
                .tenant_max_concurrent(req.tenant_id)
                .await
                .map_err(|e| Status::internal(format!("read tenant quota: {e}")))?
        } else {
            None
        };
        let trunk_name = req
            .trunk_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let trunk_max = req.trunk_max_calls.filter(|m| *m > 0);
        let trunk_max_cps = req.trunk_max_cps.filter(|m| *m > 0);
        let (granted, active, trunk_active, trunk_cps_active) = self
            .workers
            .acquire_call_slot(
                &req.call_id,
                req.tenant_id,
                max,
                trunk_name.clone(),
                trunk_max,
                trunk_max_cps,
            )
            .await
            .map_err(|e| Status::internal(format!("raft write failed: {e}")))?;
        if !granted {
            warn!(
                tenant = req.tenant_id,
                trunk = ?trunk_name,
                call_id = %req.call_id,
                active,
                max = ?max,
                trunk_active,
                trunk_max = ?trunk_max,
                trunk_cps_active,
                trunk_max_cps = ?trunk_max_cps,
                "call slot denied — tenant or trunk concurrency cap reached"
            );
        }
        Ok(Response::new(AcquireSlotResponse {
            granted,
            active,
            max: max.unwrap_or(0),
            trunk_active,
            trunk_max: trunk_max.unwrap_or(0),
            trunk_cps_active,
            trunk_cps_max: trunk_max_cps.unwrap_or(0),
        }))
    }

    // ── Platform config ───────────────────────────────────────────────────────

    async fn get_platform_config(
        &self,
        _request: Request<crate::grpc::proto::control::PlatformConfigRequest>,
    ) -> Result<Response<crate::grpc::proto::control::PlatformConfig>, Status> {
        let stun_servers = crate::settings::PlatformSettings::new(&self.store.db)
            .stun_servers()
            .await;
        let recording_policy_json = crate::settings::PlatformSettings::new(&self.store.db)
            .recording_policy_json()
            .await;
        Ok(Response::new(crate::grpc::proto::control::PlatformConfig {
            stun_servers,
            recording_policy_json,
        }))
    }

    // ── Internal: write-forwarding ────────────────────────────────────────────

    async fn propose_write(
        &self,
        request: Request<crate::grpc::proto::control::ProposeWriteRequest>,
    ) -> Result<Response<crate::grpc::proto::control::ProposeWriteResponse>, Status> {
        let req = request.into_inner();
        let cmd: crate::raft::types::RegistryCommand = serde_json::from_slice(&req.command)
            .map_err(|e| Status::invalid_argument(format!("decode command: {e}")))?;
        let resp = self
            .workers
            .apply_forwarded(cmd)
            .await
            .map_err(|e| Status::internal(format!("apply forwarded write: {e}")))?;
        Ok(Response::new(
            crate::grpc::proto::control::ProposeWriteResponse {
                known: resp.known,
                removed: resp.removed,
                granted: resp.granted,
                count: resp.count,
                trunk_count: resp.trunk_count,
                trunk_cps_count: resp.trunk_cps_count,
            },
        ))
    }
}

fn extension_affinity_key(tenant_id: Option<i64>, extension: &str) -> String {
    format!(
        "extension:{}:{}",
        tenant_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "global".to_string()),
        extension
    )
}

fn contact_q_milli(contact: &str) -> u16 {
    contact
        .split(';')
        .filter_map(|part| part.split_once('='))
        .find_map(|(key, value)| {
            key.trim()
                .eq_ignore_ascii_case("q")
                .then(|| parse_q_milli(value.trim()))
                .flatten()
        })
        .unwrap_or(1000)
}

fn parse_q_milli(value: &str) -> Option<u16> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let (whole, frac) = value.split_once('.').unwrap_or((value, ""));
    let whole = whole.parse::<u16>().ok()?;
    if whole > 1 {
        return Some(1000);
    }
    let mut frac_digits = frac
        .chars()
        .take(3)
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    while frac_digits.len() < 3 {
        frac_digits.push('0');
    }
    let frac = frac_digits.parse::<u16>().unwrap_or(0);
    Some(if whole == 1 { 1000 } else { frac.min(1000) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contact_q_milli() {
        assert_eq!(contact_q_milli("<sip:1001@host>"), 1000);
        assert_eq!(contact_q_milli("<sip:1001@host>;q=0.7"), 700);
        assert_eq!(contact_q_milli("<sip:1001@host>;expires=60;q=0.25"), 250);
        assert_eq!(contact_q_milli("<sip:1001@host>;q=1.0"), 1000);
        assert_eq!(contact_q_milli("<sip:1001@host>;q=2"), 1000);
    }
}
