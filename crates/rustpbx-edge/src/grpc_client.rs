/// gRPC client wrapper for communicating with the Control Plane.
use crate::proto::control::{
    control_plane_client::ControlPlaneClient, AclRuleList, GetAclRulesRequest,
    GetRouteRulesRequest, GetTrunkConfigsRequest, GetWorkersRequest, RouteRuleList,
    TrunkConfigList, WatchRequest, WorkerList,
};
use anyhow::{Context, Result};
use tonic::transport::Channel;
use tracing::info;

#[derive(Clone)]
pub struct GrpcControlClient {
    client: ControlPlaneClient<Channel>,
    edge_id: String,
}

impl GrpcControlClient {
    pub async fn connect(addr: &str, edge_id: String) -> Result<Self> {
        let channel = Channel::from_shared(addr.to_string())
            .context("invalid control plane address")?
            .connect()
            .await
            .context("failed to connect to control plane")?;

        info!(%addr, %edge_id, "connected to control plane");
        Ok(Self {
            client: ControlPlaneClient::new(channel),
            edge_id,
        })
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

    pub async fn get_available_workers(&mut self, tenant_id: Option<i64>) -> Result<WorkerList> {
        let resp = self
            .client
            .get_available_workers(GetWorkersRequest { tenant_id })
            .await?;
        Ok(resp.into_inner())
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
