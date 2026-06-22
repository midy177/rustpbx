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
