use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct EdgeConfig {
    /// Control Plane gRPC address to connect to
    #[serde(default = "default_control_plane_addr")]
    pub control_plane_addr: String,

    /// SIP listen address (host only, ports below)
    #[serde(default = "default_sip_addr")]
    pub sip_addr: String,

    /// SIP UDP port
    #[serde(default = "default_udp_port")]
    pub udp_port: u16,

    /// SIP TCP port (0 = disabled)
    #[serde(default)]
    pub tcp_port: u16,

    /// SIP TLS port (0 = disabled)
    #[serde(default)]
    pub tls_port: u16,

    /// Public IP advertised in SIP Contact / Via headers.
    /// Must be a fixed public IP assigned to this node.
    pub public_ip: Option<String>,

    /// RTP port range start
    #[serde(default = "default_rtp_start")]
    pub rtp_start_port: u16,

    /// RTP port range end
    #[serde(default = "default_rtp_end")]
    pub rtp_end_port: u16,

    /// Edge instance identifier (reported to Control Plane)
    #[serde(default = "default_edge_id")]
    pub edge_id: String,

    /// Region/zone label reported to the Control Plane (for the admin console).
    #[serde(default)]
    pub region: String,

    /// How often (seconds) to re-pull config from Control Plane
    #[serde(default = "default_config_poll_secs")]
    pub config_poll_secs: u64,

    /// How often (seconds) to heartbeat to the Control Plane so it can track
    /// this edge's liveness.
    #[serde(default = "default_heartbeat_secs")]
    pub heartbeat_secs: u64,

    /// Log filter
    #[serde(default = "default_log")]
    pub log: String,

    /// Trusted Worker IP/CIDR list (internal INVITEs from these sources skip ACL/Auth)
    #[serde(default)]
    pub trusted_workers: Vec<String>,

    /// EdgeWorker gRPC listen address (`host:port`). When set, the Edge serves
    /// `CallStateUpdate` so Workers can report call state out-of-band. Empty →
    /// no control channel (state still flows over SIP).
    #[serde(default)]
    pub edge_worker_addr: Option<String>,
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            control_plane_addr: default_control_plane_addr(),
            sip_addr: default_sip_addr(),
            udp_port: default_udp_port(),
            tcp_port: 0,
            tls_port: 0,
            public_ip: None,
            rtp_start_port: default_rtp_start(),
            rtp_end_port: default_rtp_end(),
            edge_id: default_edge_id(),
            region: String::new(),
            config_poll_secs: default_config_poll_secs(),
            heartbeat_secs: default_heartbeat_secs(),
            log: default_log(),
            trusted_workers: Vec::new(),
            edge_worker_addr: None,
        }
    }
}

impl EdgeConfig {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    }
}

fn default_control_plane_addr() -> String {
    "http://127.0.0.1:9090".to_string()
}
fn default_sip_addr() -> String {
    "0.0.0.0".to_string()
}
fn default_udp_port() -> u16 {
    5060
}
fn default_rtp_start() -> u16 {
    12000
}
fn default_rtp_end() -> u16 {
    42000
}
fn default_edge_id() -> String {
    format!("edge-{}", std::process::id())
}
fn default_config_poll_secs() -> u64 {
    30
}
fn default_heartbeat_secs() -> u64 {
    10
}
fn default_log() -> String {
    "info".to_string()
}
