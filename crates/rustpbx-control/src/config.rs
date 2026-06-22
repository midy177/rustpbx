use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct ControlConfig {
    /// gRPC listen address
    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: String,

    /// HTTP/Console listen address
    #[serde(default = "default_http_addr")]
    pub http_addr: String,

    /// Database URL — supports sqlite://, mysql://, postgres://
    #[serde(default = "default_database_url")]
    pub database_url: String,

    /// Super-admin username for the HTTP admin console.
    #[serde(default = "default_admin_username")]
    pub admin_username: String,

    /// Super-admin password for the HTTP admin console (CHANGE IN PRODUCTION).
    #[serde(default = "default_admin_password")]
    pub admin_password: String,

    /// Directory containing the built SPA (`web/dist`).
    #[serde(default = "default_web_dir")]
    pub web_dir: String,

    /// Log level / filter
    #[serde(default = "default_log")]
    pub log: String,

    /// Worker heartbeat timeout in seconds. A Media Worker is marked unhealthy
    /// (excluded from routing) once its heartbeat is older than this, and is
    /// reaped from the registry after twice this duration.
    /// Default 30s → unhealthy at 30s, reaped at 60s.
    #[serde(default = "default_heartbeat_timeout_secs")]
    pub heartbeat_timeout_secs: u64,

    /// Raft cluster settings for control-plane replication.
    #[serde(default)]
    pub raft: RaftConfig,

    /// Optional TLS for all gRPC (business ControlPlane + Raft transport).
    #[serde(default)]
    pub tls: TlsConfig,
}

/// TLS material for the control plane's gRPC endpoints. When `cert_path` and
/// `key_path` are both set, the business gRPC server and the Raft transport
/// server enable TLS, and the Raft/forwarding clients connect over TLS using
/// `ca_path` to verify peers. Empty (default) → plaintext (backward compatible).
#[derive(Debug, Deserialize, Clone, Default)]
pub struct TlsConfig {
    /// Server certificate chain (PEM).
    #[serde(default)]
    pub cert_path: String,
    /// Server private key (PEM).
    #[serde(default)]
    pub key_path: String,
    /// CA certificate (PEM) the clients use to verify peer servers. Defaults to
    /// `cert_path` when empty (self-signed / shared cert setups).
    #[serde(default)]
    pub ca_path: String,
    /// Domain name to verify in peer certificates. Defaults to the connection
    /// host when empty.
    #[serde(default)]
    pub domain: String,
}

impl TlsConfig {
    /// Whether TLS is configured (both cert and key present).
    pub fn is_enabled(&self) -> bool {
        !self.cert_path.trim().is_empty() && !self.key_path.trim().is_empty()
    }

    /// Build the server-side TLS config (identity from cert+key).
    pub fn server_tls(&self) -> anyhow::Result<tonic::transport::ServerTlsConfig> {
        let cert = std::fs::read(&self.cert_path)?;
        let key = std::fs::read(&self.key_path)?;
        let identity = tonic::transport::Identity::from_pem(cert, key);
        Ok(tonic::transport::ServerTlsConfig::new().identity(identity))
    }

    /// Build the client-side TLS config (CA to verify peers, optional domain).
    pub fn client_tls(&self) -> anyhow::Result<tonic::transport::ClientTlsConfig> {
        let ca_path = if self.ca_path.trim().is_empty() {
            &self.cert_path
        } else {
            &self.ca_path
        };
        let ca = std::fs::read(ca_path)?;
        let mut cfg = tonic::transport::ClientTlsConfig::new()
            .ca_certificate(tonic::transport::Certificate::from_pem(ca));
        if !self.domain.trim().is_empty() {
            cfg = cfg.domain_name(self.domain.clone());
        }
        Ok(cfg)
    }
}

/// Raft replication config for the worker registry.
///
/// With no `addr` configured the node runs as a single-voter cluster (Phase 1
/// behaviour, fully backward compatible). Setting `addr` starts a dedicated
/// Raft gRPC server so this node can join a multi-replica cluster; peers are
/// added dynamically at runtime via the admin API (`add_learner` +
/// `change_membership`).
#[derive(Debug, Deserialize, Clone)]
pub struct RaftConfig {
    /// This node's Raft id. Must be unique and stable across the cluster.
    #[serde(default = "default_raft_node_id")]
    pub node_id: u64,

    /// Dedicated address for inter-node Raft traffic, e.g. `0.0.0.0:9091`.
    /// Empty (default) → single-node mode, no Raft server started.
    #[serde(default)]
    pub addr: String,

    /// Address other replicas use to reach this node's Raft server, e.g.
    /// `10.0.0.7:9091`. Advertised when joining a cluster. Defaults to `addr`
    /// if empty.
    #[serde(default)]
    pub advertise_addr: String,
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            node_id: default_raft_node_id(),
            addr: String::new(),
            advertise_addr: String::new(),
        }
    }
}

impl RaftConfig {
    /// Whether this node should run a Raft server and operate in cluster mode.
    pub fn is_cluster_mode(&self) -> bool {
        !self.addr.trim().is_empty()
    }

    /// The address to advertise to peers (falls back to `addr`).
    pub fn effective_advertise_addr(&self) -> &str {
        if self.advertise_addr.trim().is_empty() {
            &self.addr
        } else {
            &self.advertise_addr
        }
    }
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            grpc_addr: default_grpc_addr(),
            http_addr: default_http_addr(),
            database_url: default_database_url(),
            admin_username: default_admin_username(),
            admin_password: default_admin_password(),
            web_dir: default_web_dir(),
            log: default_log(),
            heartbeat_timeout_secs: default_heartbeat_timeout_secs(),
            raft: RaftConfig::default(),
            tls: TlsConfig::default(),
        }
    }
}

impl ControlConfig {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&contents)?;
        Ok(config)
    }
}

fn default_grpc_addr() -> String {
    "0.0.0.0:9090".to_string()
}

fn default_http_addr() -> String {
    "0.0.0.0:9080".to_string()
}

fn default_database_url() -> String {
    "sqlite://rustpbx-control.sqlite3".to_string()
}

fn default_admin_username() -> String {
    "admin".to_string()
}

fn default_admin_password() -> String {
    "admin".to_string()
}

fn default_web_dir() -> String {
    "crates/rustpbx-control/web/dist".to_string()
}

fn default_log() -> String {
    "info".to_string()
}

fn default_heartbeat_timeout_secs() -> u64 {
    30
}

fn default_raft_node_id() -> u64 {
    1
}
