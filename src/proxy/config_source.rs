use crate::proxy::routing::{RouteRule, TrunkConfig};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

/// An event emitted by ProxyDataContext after a successful trunk reload.
#[derive(Debug, Clone)]
pub enum ConfigChangeEvent {
    TrunkAdded {
        name: String,
        config: TrunkConfig,
    },
    TrunkUpdated {
        name: String,
        config: TrunkConfig,
    },
    TrunkRemoved {
        name: String,
    },
    RoutesReloaded {
        rules: Vec<RouteRule>,
    },
    AclReloaded {
        rules: Vec<String>,
    },
}

/// Abstraction over the source of trunk / route / ACL configuration.
///
/// The current implementation (`LocalConfigSource`) loads from embedded
/// config + TOML files + database.  Future implementations will fetch from
/// the Control Plane gRPC service.
#[async_trait]
pub trait ConfigSource: Send + Sync {
    async fn load_trunks(&self) -> Result<HashMap<String, TrunkConfig>>;
    async fn load_routes(&self) -> Result<Vec<RouteRule>>;
    async fn load_acl_rules(&self) -> Result<Vec<String>>;
}
