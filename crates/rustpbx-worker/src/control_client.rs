/// Worker-side Control Plane client.
///
/// Handles:
///  - RegisterWorker on startup
///  - WorkerHeartbeat on interval
///  - ReportCallRecord after each call
use crate::{
    config::WorkerConfig,
    proto::control::{
        CallRecordReport, HeartbeatRequest, RegisterAck, WorkerInfo,
        control_plane_client::ControlPlaneClient,
    },
};
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use tokio::time::{Duration, interval};
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;
use tracing::{info, warn};

pub struct ControlClient {
    client: ControlPlaneClient<Channel>,
    worker_id: String,
    sip_addr: String,
    rtp_external_ip: String,
    rtp_start_port: u32,
    rtp_end_port: u32,
    max_concurrent: u32,
    labels: HashMap<String, String>,
    capabilities: Vec<String>,
    /// EdgeWorker gRPC addr advertised for AllocateCall (empty if disabled).
    edge_worker_addr: String,
    /// Detected NAT type (STUN), reported at registration. Set by main.
    pub nat_type: String,
    /// Shared counter updated by call sessions
    pub active_calls: Arc<AtomicU32>,
}

impl ControlClient {
    pub async fn connect(cfg: &WorkerConfig) -> Result<Self> {
        let tls = cfg.tls.load()?;
        let channel = rustpbx_proto::tls::endpoint(&cfg.control_plane_addr, tls.as_ref())
            .map_err(|e| anyhow::anyhow!("invalid control plane addr/TLS: {e}"))?
            .connect()
            .await
            .context("connect to control plane")?;

        info!(addr = %cfg.control_plane_addr, worker_id = %cfg.worker_id, tls = tls.is_some(), "connected to control plane");

        Ok(Self {
            client: ControlPlaneClient::new(channel),
            worker_id: cfg.worker_id.clone(),
            sip_addr: format!("{}:{}", cfg.sip_addr, cfg.sip_port),
            rtp_external_ip: cfg
                .rtp_external_ip
                .clone()
                .unwrap_or_else(|| cfg.rtp_bind_ip.clone()),
            rtp_start_port: cfg.rtp_start_port as u32,
            rtp_end_port: cfg.rtp_end_port as u32,
            max_concurrent: cfg.max_concurrent,
            labels: cfg.labels.clone(),
            capabilities: cfg.capabilities.clone(),
            edge_worker_addr: cfg.edge_worker_addr.clone().unwrap_or_default(),
            nat_type: String::new(),
            active_calls: Arc::new(AtomicU32::new(0)),
        })
    }

    /// Replace wildcard (0.0.0.0 / ::) hosts in the addresses this worker
    /// reports with the STUN-detected public IP, so a worker that listens on
    /// 0.0.0.0 (hostNetwork / containers) advertises a reachable SIP +
    /// AllocateCall address without per-node config. Applies to `sip_addr` and
    /// `edge_worker_addr`; call after the STUN probe, before `register`.
    pub fn apply_detected_public_ip(&mut self, public_ip: &Option<String>) {
        let Some(ip) = public_ip.as_deref() else {
            return;
        };
        self.sip_addr = with_public_host(&self.sip_addr, ip);
        if !self.edge_worker_addr.is_empty() {
            self.edge_worker_addr = with_public_host(&self.edge_worker_addr, ip);
        }
    }

    /// Connect, retrying with exponential backoff until the control plane is
    /// reachable — so a worker started before the control plane waits instead of
    /// crashing.
    pub async fn connect_with_retry(cfg: &WorkerConfig) -> Result<Self> {
        // Validate the address + TLS material once up front so a malformed
        // address or missing cert fails fast instead of retrying forever.
        let tls = cfg.tls.load()?;
        rustpbx_proto::tls::endpoint(&cfg.control_plane_addr, tls.as_ref())
            .map_err(|e| anyhow::anyhow!("invalid control plane addr/TLS: {e}"))?;

        let mut delay = Duration::from_millis(500);
        loop {
            match Self::connect(cfg).await {
                Ok(c) => return Ok(c),
                Err(e) => {
                    warn!(error = %e, ?delay, "control plane unreachable; retrying");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(15));
                }
            }
        }
    }

    /// Register this worker with the Control Plane on startup.
    pub async fn register(&mut self) -> Result<RegisterAck> {
        let resp = self
            .client
            .register_worker(WorkerInfo {
                worker_id: self.worker_id.clone(),
                sip_addr: self.sip_addr.clone(),
                rtp_external_ip: self.rtp_external_ip.clone(),
                rtp_start_port: self.rtp_start_port,
                rtp_end_port: self.rtp_end_port,
                max_concurrent: self.max_concurrent,
                active_calls: self.active_calls.load(Ordering::Relaxed),
                labels: self.labels.clone(),
                capabilities: self.capabilities.clone(),
                edge_worker_addr: self.edge_worker_addr.clone(),
                nat_type: self.nat_type.clone(),
            })
            .await?;
        Ok(resp.into_inner())
    }

    /// Send a single heartbeat.
    pub async fn heartbeat(&mut self) -> Result<bool> {
        let active = self.active_calls.load(Ordering::Relaxed);
        let resp = self
            .client
            .worker_heartbeat(HeartbeatRequest {
                worker_id: self.worker_id.clone(),
                active_calls: active,
                cpu_usage: cpu_usage_approx(),
                rtp_ports_used: active * 2, // rough estimate: 2 ports per call
            })
            .await?;
        let inner = resp.into_inner();
        if inner.drain {
            warn!("control plane asked worker to drain — no new calls will be accepted");
        }
        Ok(!inner.drain)
    }

    /// Upload a completed CDR to the Control Plane.
    pub async fn report_cdr(&mut self, report: CallRecordReport) -> Result<()> {
        let resp = self.client.report_call_record(report).await?;
        if !resp.into_inner().accepted {
            warn!("control plane rejected CDR");
        }
        Ok(())
    }

    /// Increment the active call counter (call when INVITE is answered).
    #[allow(dead_code)]
    pub fn inc_active(&self) {
        self.active_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the active call counter (call when BYE is processed).
    #[allow(dead_code)]
    pub fn dec_active(&self) {
        self.active_calls.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Run the heartbeat loop until cancelled.
pub async fn run_heartbeat(
    mut client: ControlClient,
    interval_secs: u64,
    cancel: CancellationToken,
) {
    let mut ticker = interval(Duration::from_secs(interval_secs));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(e) = client.heartbeat().await {
                    warn!(error = %e, "heartbeat failed");
                }
            }
            _ = cancel.cancelled() => {
                info!("heartbeat loop stopped");
                break;
            }
        }
    }
}

/// Fetch the centrally-managed STUN list from the control plane (superadmin →
/// platform settings). Returns empty on any error — the caller falls back to
/// the node's local `stun_servers` config.
pub async fn fetch_platform_stun(
    control_plane_addr: &str,
    tls: Option<&rustpbx_proto::tls::ClientTls>,
) -> Vec<String> {
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

/// Fetch the active IVR flows from the control plane. Returns `(name,
/// spec_json)` pairs — spec is an opaque `IvrDefinition` JSON; the worker
/// materializes it to a TOML file. Empty on any error.
pub async fn fetch_ivrs(
    control_plane_addr: &str,
    tls: Option<&rustpbx_proto::tls::ClientTls>,
) -> Vec<(String, String)> {
    use crate::proto::control::{GetIvrsRequest, control_plane_client::ControlPlaneClient};
    let Ok(ep) = rustpbx_proto::tls::endpoint(control_plane_addr, tls) else {
        return Vec::new();
    };
    let Ok(channel) = ep.connect().await else {
        return Vec::new();
    };
    let Ok(resp) = ControlPlaneClient::new(channel)
        .get_ivrs(GetIvrsRequest { tenant_id: None })
        .await
    else {
        return Vec::new();
    };
    resp.into_inner()
        .ivrs
        .into_iter()
        .map(|i| (i.name, i.spec_json))
        .collect()
}

/// Write IVR definitions to `{ivr_dir}/{name}.generated.toml`. Each spec_json
/// is deserialized into an IvrDefinition, wrapped in IvrFileConfig, and
/// serialized to TOML — the format the shared CallModule reads at runtime
/// (sip_session.rs reads these files). Returns the count materialized;
/// invalid specs are skipped with a warning.
pub async fn materialize_ivrs(ivrs: &[(String, String)], ivr_dir: &std::path::Path) -> usize {
    use rustpbx::call::app::ivr_config::{IvrDefinition, IvrFileConfig};
    if let Err(e) = tokio::fs::create_dir_all(ivr_dir).await {
        warn!(dir = %ivr_dir.display(), error = %e, "cannot create ivr_dir");
        return 0;
    }
    let mut ok = 0usize;
    for (name, spec_json) in ivrs {
        let spec_json = spec_json.trim();
        if spec_json.is_empty() {
            continue;
        }
        let result = async {
            let def: IvrDefinition = serde_json::from_str(spec_json)?;
            let fc = IvrFileConfig { ivr: def };
            let toml_str = toml::to_string_pretty(&fc)?;
            let path = ivr_dir.join(format!("{name}.generated.toml"));
            tokio::fs::write(&path, toml_str).await?;
            anyhow::Ok(path)
        }
        .await;
        match result {
            Ok(_) => ok += 1,
            Err(e) => warn!(name = %name, error = %e, "failed to materialize IVR"),
        }
    }
    ok
}

/// Fetch the global call-recording policy from the control plane and
/// deserialize it into a `RecordingPolicy`. Returns None on any error or when
/// recording is disabled — the worker then has no global recording policy.
pub async fn fetch_platform_recording(
    control_plane_addr: &str,
    tls: Option<&rustpbx_proto::tls::ClientTls>,
) -> Option<rustpbx::config::RecordingPolicy> {
    use crate::proto::control::{PlatformConfigRequest, control_plane_client::ControlPlaneClient};
    let Ok(ep) = rustpbx_proto::tls::endpoint(control_plane_addr, tls) else {
        return None;
    };
    let Ok(channel) = ep.connect().await else {
        return None;
    };
    let json = ControlPlaneClient::new(channel)
        .get_platform_config(PlatformConfigRequest {})
        .await
        .ok()?
        .into_inner()
        .recording_policy_json?;
    let json = json.trim();
    if json.is_empty() {
        return None;
    }
    match serde_json::from_str::<rustpbx::config::RecordingPolicy>(json) {
        Ok(p) if p.enabled.unwrap_or(false) => Some(p),
        Ok(_) => None, // configured but disabled
        Err(e) => {
            tracing::warn!(error = %e, "invalid recording_policy_json from control plane");
            None
        }
    }
}

/// Fetch the active call queues from the control plane and deserialize each
/// `spec_json` into a `RouteQueueConfig`. Returns an empty map on any error —
/// the worker then simply has no queues until the next pull. Worker serves all
/// tenants, so this loads every active queue.
pub async fn fetch_queues(
    control_plane_addr: &str,
    tls: Option<&rustpbx_proto::tls::ClientTls>,
) -> std::collections::HashMap<String, rustpbx::proxy::routing::RouteQueueConfig> {
    use crate::proto::control::{GetQueuesRequest, control_plane_client::ControlPlaneClient};
    use rustpbx::proxy::routing::RouteQueueConfig;
    let Ok(ep) = rustpbx_proto::tls::endpoint(control_plane_addr, tls) else {
        return Default::default();
    };
    let Ok(channel) = ep.connect().await else {
        return Default::default();
    };
    let Ok(resp) = ControlPlaneClient::new(channel)
        .get_queues(GetQueuesRequest { tenant_id: None })
        .await
    else {
        return Default::default();
    };
    let mut map = std::collections::HashMap::new();
    for q in resp.into_inner().queues {
        match serde_json::from_str::<RouteQueueConfig>(&q.spec_json) {
            Ok(cfg) => {
                map.insert(q.name, cfg);
            }
            Err(e) => {
                tracing::warn!(name = %q.name, error = %e, "skipped queue: invalid spec_json")
            }
        }
    }
    map
}

fn cpu_usage_approx() -> f32 {
    use std::sync::Mutex;
    use sysinfo::System;
    static SYS: std::sync::OnceLock<Mutex<System>> = std::sync::OnceLock::new();
    let mut guard = SYS
        .get_or_init(|| Mutex::new(System::new()))
        .lock()
        .unwrap();
    guard.refresh_cpu_usage();
    guard.global_cpu_usage()
}

/// If `addr` parses to a SocketAddr bound to an unspecified IP (0.0.0.0 / ::),
/// swap in `public_ip` while keeping the port; otherwise return it unchanged.
fn with_public_host(addr: &str, public_ip: &str) -> String {
    match addr.parse::<std::net::SocketAddr>() {
        Ok(sa) if sa.ip().is_unspecified() => format!("{public_ip}:{}", sa.port()),
        _ => addr.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_host_is_replaced_with_public_ip() {
        assert_eq!(
            with_public_host("0.0.0.0:5070", "203.0.113.20"),
            "203.0.113.20:5070"
        );
        assert_eq!(
            with_public_host("0.0.0.0:9092", "203.0.113.20"),
            "203.0.113.20:9092"
        );
    }

    #[test]
    fn explicit_host_is_left_alone() {
        assert_eq!(
            with_public_host("10.0.0.5:5070", "203.0.113.20"),
            "10.0.0.5:5070"
        );
        // Already-public address is not rewritten.
        assert_eq!(
            with_public_host("203.0.113.99:5070", "203.0.113.20"),
            "203.0.113.99:5070"
        );
    }

    #[test]
    fn malformed_address_is_left_alone() {
        assert_eq!(
            with_public_host("not-an-addr", "203.0.113.20"),
            "not-an-addr"
        );
    }
}
