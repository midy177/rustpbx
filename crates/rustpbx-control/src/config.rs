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

    /// Log level / filter
    #[serde(default = "default_log")]
    pub log: String,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            grpc_addr: default_grpc_addr(),
            http_addr: default_http_addr(),
            database_url: default_database_url(),
            log: default_log(),
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

fn default_log() -> String {
    "info".to_string()
}
