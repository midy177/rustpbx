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

    /// STUN servers (`host:port`) for public-IP + NAT-type detection on startup.
    /// Two different servers enable cone-vs-symmetric classification.
    #[serde(default = "default_stun_servers")]
    pub stun_servers: Vec<String>,

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

    /// HTTP health endpoint listen address (`host:port`, e.g. "0.0.0.0:8081").
    /// Serves GET /healthz (liveness) and /readyz (ready once registered with
    /// the Control Plane). Empty → no health server (use a tcpSocket probe).
    #[serde(default)]
    pub health_addr: Option<String>,

    /// TLS for the Control Plane gRPC connection. Empty → plaintext. Use an
    /// `https://` `control_plane_addr` when this is set.
    #[serde(default)]
    pub tls: TlsClientConfig,
}

/// Client-side TLS material for connecting to the Control Plane. Setting
/// `ca_path` enables TLS (verify the server cert); adding a client cert + key
/// turns on mutual TLS so the Control Plane can authenticate this node.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct TlsClientConfig {
    /// CA cert (PEM) used to verify the Control Plane's server cert. Empty →
    /// TLS disabled.
    #[serde(default)]
    pub ca_path: String,
    /// Client cert (PEM) presented for mutual TLS. Optional.
    #[serde(default)]
    pub client_cert_path: String,
    /// Client private key (PEM) for mutual TLS. Optional.
    #[serde(default)]
    pub client_key_path: String,
    /// Domain to verify against the server cert SAN (override; useful when the
    /// address is an IP). Empty → derived from the address host.
    #[serde(default)]
    pub domain: String,
}

impl TlsClientConfig {
    pub fn is_enabled(&self) -> bool {
        !self.ca_path.trim().is_empty()
    }

    /// Load the PEM files into a shared `ClientTls`, or `None` when TLS isn't
    /// configured (plaintext, backward compatible).
    pub fn load(&self) -> anyhow::Result<Option<rustpbx_proto::tls::ClientTls>> {
        if !self.is_enabled() {
            return Ok(None);
        }
        let ca_pem = std::fs::read(&self.ca_path)?;
        let identity = if !self.client_cert_path.trim().is_empty()
            && !self.client_key_path.trim().is_empty()
        {
            Some((
                std::fs::read(&self.client_cert_path)?,
                std::fs::read(&self.client_key_path)?,
            ))
        } else {
            None
        };
        let domain = (!self.domain.trim().is_empty()).then(|| self.domain.clone());
        Ok(Some(rustpbx_proto::tls::ClientTls { ca_pem, identity, domain }))
    }
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
            stun_servers: default_stun_servers(),
            log: default_log(),
            trusted_workers: Vec::new(),
            edge_worker_addr: None,
            health_addr: None,
            tls: TlsClientConfig::default(),
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
fn default_stun_servers() -> Vec<String> {
    vec![
        "stun.l.google.com:19302".to_string(),
        "stun1.l.google.com:19302".to_string(),
    ]
}
fn default_log() -> String {
    "info".to_string()
}
