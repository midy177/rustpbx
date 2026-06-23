use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct WorkerConfig {
    /// Control Plane gRPC address
    #[serde(default = "default_control_plane_addr")]
    pub control_plane_addr: String,

    /// SIP listen address (internal, from Edge)
    #[serde(default = "default_sip_addr")]
    pub sip_addr: String,

    /// SIP UDP port for internal signaling (Edge → Worker)
    #[serde(default = "default_sip_port")]
    pub sip_port: u16,

    /// Public/external IP for RTP (must be reachable by PSTN carriers via Edge)
    pub rtp_external_ip: Option<String>,

    /// RTP bind IP (usually 0.0.0.0)
    #[serde(default = "default_rtp_bind")]
    pub rtp_bind_ip: String,

    /// RTP port range
    #[serde(default = "default_rtp_start")]
    pub rtp_start_port: u16,
    #[serde(default = "default_rtp_end")]
    pub rtp_end_port: u16,

    /// Max concurrent calls this worker accepts
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,

    /// Worker instance ID (defaults to hostname:pid)
    #[serde(default = "default_worker_id")]
    pub worker_id: String,

    /// Database URL — Worker uses it for IVR / queue state if needed.
    /// Can be empty if Worker operates in stateless mode.
    #[serde(default = "default_database_url")]
    #[allow(dead_code)]
    pub database_url: String,

    /// Recording root directory
    #[serde(default = "default_recording_path")]
    #[allow(dead_code)]
    pub recording_path: String,

    /// Heartbeat interval in seconds
    #[serde(default = "default_heartbeat_secs")]
    pub heartbeat_secs: u64,

    /// Prometheus metrics listen address (empty = disabled)
    #[serde(default)]
    pub metrics_addr: Option<String>,

    /// Trusted Edge IP/CIDR list — SIP INVITEs from these sources are treated
    /// as internal calls (bypass ACL/Auth, decode X-* routing headers).
    /// Each entry is an IP ("10.0.0.3") or CIDR ("10.0.0.0/24").
    #[serde(default)]
    pub trusted_edges: Vec<String>,

    /// STUN servers (`host:port`) for public-IP + NAT-type detection on startup.
    /// Two different servers enable cone-vs-symmetric classification. Empty →
    /// skip the probe (nat_type reported as "unknown").
    #[serde(default = "default_stun_servers")]
    pub stun_servers: Vec<String>,

    /// SIP address of the Edge to which outbound calls are forwarded
    /// (`host:port`, e.g. "10.0.0.3:5060"). When unset, the Worker rejects
    /// outbound origination from local extensions (worker-only inbound mode).
    /// The Worker's source IP must appear in the target Edge's `trusted_workers`.
    #[serde(default)]
    pub edge_sip_addr: Option<String>,

    /// EdgeWorker gRPC listen address (`host:port`, e.g. "0.0.0.0:9092"). When
    /// set, the Worker serves `AllocateCall` so the Edge can reserve a slot
    /// before sending the INVITE. Empty → the Edge falls back to selecting via
    /// the Control Plane's worker list without reservation.
    #[serde(default)]
    pub edge_worker_addr: Option<String>,

    /// SIP contact advertised to the Edge in `AllocateCall` responses
    /// (`sip:host:port`). Defaults to `sip:{sip_addr}:{sip_port}` when unset.
    #[serde(default)]
    pub sip_contact: Option<String>,

    /// Edge's EdgeWorker gRPC address (`host:port`) that this Worker reports
    /// `CallStateUpdate` events to. Empty → no out-of-band state reporting
    /// (state still flows over SIP).
    #[serde(default)]
    pub edge_report_addr: Option<String>,

    /// HTTP health endpoint listen address (`host:port`, e.g. "0.0.0.0:8081").
    /// Serves GET /healthz (liveness) and /readyz (ready once registered with
    /// the Control Plane). Empty → no health server (use a tcpSocket probe).
    #[serde(default)]
    pub health_addr: Option<String>,

    #[serde(default = "default_log")]
    pub log: String,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            control_plane_addr: default_control_plane_addr(),
            sip_addr: default_sip_addr(),
            sip_port: default_sip_port(),
            rtp_external_ip: None,
            rtp_bind_ip: default_rtp_bind(),
            rtp_start_port: default_rtp_start(),
            rtp_end_port: default_rtp_end(),
            max_concurrent: default_max_concurrent(),
            worker_id: default_worker_id(),
            database_url: default_database_url(),
            recording_path: default_recording_path(),
            heartbeat_secs: default_heartbeat_secs(),
            metrics_addr: None,
            trusted_edges: Vec::new(),
            stun_servers: default_stun_servers(),
            edge_sip_addr: None,
            edge_worker_addr: None,
            sip_contact: None,
            edge_report_addr: None,
            health_addr: None,
            log: default_log(),
        }
    }
}

impl WorkerConfig {
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
fn default_sip_port() -> u16 {
    5070 // different from Edge's 5060 when co-located
}
fn default_rtp_bind() -> String {
    "0.0.0.0".to_string()
}
fn default_rtp_start() -> u16 {
    12000
}
fn default_rtp_end() -> u16 {
    42000
}
fn default_max_concurrent() -> u32 {
    100
}
fn default_worker_id() -> String {
    format!(
        "worker-{}",
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| std::process::id().to_string())
    )
}
fn default_database_url() -> String {
    "sqlite://rustpbx-worker.sqlite3".to_string()
}
fn default_recording_path() -> String {
    "./recordings".to_string()
}
fn default_stun_servers() -> Vec<String> {
    vec![
        "stun.l.google.com:19302".to_string(),
        "stun1.l.google.com:19302".to_string(),
    ]
}

fn default_heartbeat_secs() -> u64 {
    10
}
fn default_log() -> String {
    "info".to_string()
}
