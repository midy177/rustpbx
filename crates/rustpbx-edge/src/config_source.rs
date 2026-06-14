/// GrpcConfigSource: fetches trunk/route/ACL config from the Control Plane.
///
/// Defines a local ConfigSource trait (mirrors the one in src/proxy/config_source.rs
/// which is not exposed as public API from the main crate).
use crate::grpc_client::GrpcControlClient;
use anyhow::Result;
use async_trait::async_trait;
use rustpbx::proxy::routing::{
    DestConfig, MatchConditions, RouteAction, RouteRule, TrunkConfig,
};
use std::collections::HashMap;
use tokio::sync::RwLock;
use std::sync::Arc;
use tracing::info;

// ── Local ConfigSource trait ──────────────────────────────────────────────────
// Mirrors src/proxy/config_source.rs (not pub from main crate).

#[async_trait]
pub trait ConfigSource: Send + Sync {
    async fn load_trunks(&self) -> Result<HashMap<String, TrunkConfig>>;
    async fn load_routes(&self) -> Result<Vec<RouteRule>>;
    async fn load_acl_rules(&self) -> Result<Vec<String>>;
}

// ── GrpcConfigSource ──────────────────────────────────────────────────────────

pub struct GrpcConfigSource {
    client: Arc<RwLock<GrpcControlClient>>,
    tenant_id: Option<i64>,
}

impl GrpcConfigSource {
    pub fn new(client: GrpcControlClient, tenant_id: Option<i64>) -> Self {
        Self {
            client: Arc::new(RwLock::new(client)),
            tenant_id,
        }
    }
}

#[async_trait]
impl ConfigSource for GrpcConfigSource {
    async fn load_trunks(&self) -> Result<HashMap<String, TrunkConfig>> {
        let mut client = self.client.write().await;
        let list = client.get_trunk_configs(self.tenant_id).await?;
        info!(count = list.trunks.len(), version = list.version, "pulled trunks");
        Ok(list.trunks.into_iter().map(trunk_from_proto).collect())
    }

    async fn load_routes(&self) -> Result<Vec<RouteRule>> {
        let mut client = self.client.write().await;
        let list = client.get_route_rules(self.tenant_id).await?;
        info!(count = list.rules.len(), "pulled routes");
        Ok(list.rules.into_iter().filter_map(route_from_proto).collect())
    }

    async fn load_acl_rules(&self) -> Result<Vec<String>> {
        let mut client = self.client.write().await;
        let list = client.get_acl_rules(self.tenant_id).await?;
        info!(count = list.rules.len(), "pulled acl rules");
        Ok(list.rules)
    }
}

// ── Converters ────────────────────────────────────────────────────────────────

fn trunk_from_proto(p: crate::proto::control::TrunkConfigProto) -> (String, TrunkConfig) {
    use rustpbx::proxy::routing::TrunkDirection;
    let direction = p.direction.as_deref().and_then(|d| match d {
        "inbound" => Some(TrunkDirection::Inbound),
        "outbound" => Some(TrunkDirection::Outbound),
        "bidirectional" => Some(TrunkDirection::Bidirectional),
        _ => None,
    });
    let trunk = TrunkConfig {
        dest: p.dest,
        backup_dest: p.backup_dest,
        username: p.username,
        password: p.password,
        codec: p.codec,
        id: p.id,
        direction,
        inbound_hosts: p.inbound_hosts,
        did_numbers: p.did_numbers,
        register_enabled: p.register_enabled,
        register_expires: p.register_expires,
        register_extra_headers: if p.register_extra_headers.is_empty() {
            None
        } else {
            Some(p.register_extra_headers)
        },
        rewrite_hostport: p.rewrite_hostport.unwrap_or(true),
        incoming_from_user_prefix: p.incoming_from_user_prefix,
        incoming_to_user_prefix: p.incoming_to_user_prefix,
        country: p.country,
        ..Default::default()
    };
    (p.name, trunk)
}

fn route_from_proto(p: crate::proto::control::RouteRuleProto) -> Option<RouteRule> {
    let mc = p.match_conditions.map(|m| MatchConditions {
        from_user: m.from_user,
        from_host: m.from_host,
        to_user: m.to_user,
        to_host: m.to_host,
        from: m.from,
        to: m.to,
        caller: m.caller,
        callee: m.callee,
        headers: m.headers,
        ..Default::default()
    }).unwrap_or_default();

    let dest = match p.action.as_ref().map(|a| a.dest.as_slice()) {
        Some([s]) => Some(DestConfig::Single(s.clone())),
        Some(m) if m.len() > 1 => Some(DestConfig::Multiple(m.to_vec())),
        _ => None,
    };
    let action = RouteAction {
        dest,
        queue: p.action.as_ref().and_then(|a| a.queue.clone()),
        app: p.action.as_ref().and_then(|a| a.app.clone()),
        auto_answer: p.action.as_ref().map(|a| a.auto_answer).unwrap_or(true),
        ..Default::default()
    };

    Some(RouteRule {
        name: p.name,
        description: p.description,
        priority: p.priority,
        source_trunks: p.source_trunks,
        match_conditions: mc,
        action,
        disabled: p.disabled,
        ..Default::default()
    })
}
