/// gRPC client wrapper for communicating with the Control Plane.
use crate::proto::control::{
    AclRuleList, EdgeHeartbeatRequest, EdgeInfo, GetAclRulesRequest, GetRouteRulesRequest,
    GetTrunkConfigsRequest, GetWorkersRequest, RegisterAck, RouteRuleList, TrunkConfigList,
    WatchRequest, WorkerList, control_plane_client::ControlPlaneClient,
};
use anyhow::{Context, Result};
use rustpbx_proto::tls::ClientTls;
use std::collections::HashMap;
use tonic::transport::Channel;
use tracing::{info, warn};

#[derive(Clone)]
pub struct GrpcControlClient {
    client: ControlPlaneClient<Channel>,
    edge_id: String,
}

impl GrpcControlClient {
    pub async fn connect(addr: &str, edge_id: String, tls: Option<&ClientTls>) -> Result<Self> {
        let channel = rustpbx_proto::tls::endpoint(addr, tls)
            .map_err(|e| anyhow::anyhow!("invalid control plane address/TLS: {e}"))?
            .connect()
            .await
            .context("failed to connect to control plane")?;

        info!(%addr, %edge_id, tls = tls.is_some(), "connected to control plane");
        Ok(Self {
            client: ControlPlaneClient::new(channel),
            edge_id,
        })
    }

    /// Connect, retrying with exponential backoff until the control plane is
    /// reachable — so a node started before the control plane waits instead of
    /// crashing. (An invalid address/TLS config fails fast.)
    pub async fn connect_with_retry(
        addr: &str,
        edge_id: String,
        tls: Option<&ClientTls>,
    ) -> Result<Self> {
        let mut delay = std::time::Duration::from_millis(500);
        loop {
            match Self::connect(addr, edge_id.clone(), tls).await {
                Ok(c) => return Ok(c),
                Err(e) => {
                    // A malformed address/TLS config never becomes valid — don't
                    // loop forever.
                    if rustpbx_proto::tls::endpoint(addr, tls).is_err() {
                        return Err(e);
                    }
                    tracing::warn!(error = %e, ?delay, "control plane unreachable; retrying");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(std::time::Duration::from_secs(15));
                }
            }
        }
    }

    pub async fn get_trunk_configs(&mut self, tenant_id: Option<i64>) -> Result<TrunkConfigList> {
        let resp = self
            .client
            .get_trunk_configs(GetTrunkConfigsRequest {
                tenant_id,
                edge_id: Some(self.edge_id.clone()),
            })
            .await?;
        Ok(resp.into_inner())
    }

    pub async fn get_route_rules(&mut self, tenant_id: Option<i64>) -> Result<RouteRuleList> {
        let resp = self
            .client
            .get_route_rules(GetRouteRulesRequest { tenant_id })
            .await?;
        Ok(resp.into_inner())
    }

    pub async fn get_acl_rules(&mut self, tenant_id: Option<i64>) -> Result<AclRuleList> {
        let resp = self
            .client
            .get_acl_rules(GetAclRulesRequest { tenant_id })
            .await?;
        Ok(resp.into_inner())
    }

    pub async fn get_available_workers(
        &mut self,
        tenant_id: Option<i64>,
        required_labels: HashMap<String, String>,
    ) -> Result<WorkerList> {
        let resp = self
            .client
            .get_available_workers(GetWorkersRequest {
                tenant_id,
                required_labels,
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Reserve a per-tenant concurrency slot for `call_id` before forwarding an
    /// inbound INVITE. Returns `(granted, active, max)`: when `granted` is false
    /// the tenant is at its `max_concurrent_calls` cap and the call should be
    /// rejected. The slot is released when the call's CDR reaches the control
    /// plane (or reaped after a TTL).
    pub async fn acquire_call_slot(
        &mut self,
        tenant_id: i64,
        call_id: &str,
    ) -> Result<(bool, u32, u32)> {
        use crate::proto::control::AcquireSlotRequest;
        let resp = self
            .client
            .acquire_call_slot(AcquireSlotRequest {
                tenant_id,
                call_id: call_id.to_string(),
            })
            .await?
            .into_inner();
        Ok((resp.granted, resp.active, resp.max))
    }

    /// Register this edge with the Control Plane (observability only).
    pub async fn register_edge(&mut self, info: EdgeInfo) -> Result<RegisterAck> {
        let resp = self.client.register_edge(info).await?;
        Ok(resp.into_inner())
    }

    /// Send one heartbeat. Returns false if the control plane asked us to
    /// re-register (it doesn't know this edge — e.g. it restarted).
    pub async fn edge_heartbeat(&mut self, active_calls: u32) -> Result<bool> {
        let resp = self
            .client
            .edge_heartbeat(EdgeHeartbeatRequest {
                edge_id: self.edge_id.clone(),
                active_calls,
            })
            .await?;
        let drain = resp.into_inner().drain;
        if drain {
            warn!("control plane doesn't know this edge — will re-register");
        }
        Ok(!drain)
    }

    pub async fn watch_config_changes(
        &mut self,
        from_version: u64,
    ) -> Result<tonic::Streaming<crate::proto::control::ConfigChangeEvent>> {
        let resp = self
            .client
            .watch_config_changes(WatchRequest {
                edge_id: Some(self.edge_id.clone()),
                worker_id: None,
                from_version,
            })
            .await?;
        Ok(resp.into_inner())
    }
}

/// Fetch the centrally-managed STUN list from the control plane (superadmin →
/// platform settings). Returns empty on any error — the caller then falls back
/// to the node's local `stun_servers` config.
pub async fn fetch_platform_stun(control_plane_addr: &str, tls: Option<&ClientTls>) -> Vec<String> {
    use crate::proto::control::{PlatformConfigRequest, control_plane_client::ControlPlaneClient};
    let Ok(ep) = rustpbx_proto::tls::endpoint(control_plane_addr, tls) else {
        return Vec::new();
    };
    let Ok(channel) = ep.connect().await else {
        return Vec::new();
    };
    match ControlPlaneClient::new(channel)
        .get_platform_config(PlatformConfigRequest {})
        .await
    {
        Ok(r) => r.into_inner().stun_servers,
        Err(_) => Vec::new(),
    }
}
