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
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
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
    /// EdgeWorker gRPC addr advertised for AllocateCall (empty if disabled).
    edge_worker_addr: String,
    /// Detected NAT type (STUN), reported at registration. Set by main.
    pub nat_type: String,
    /// Shared counter updated by call sessions
    pub active_calls: Arc<AtomicU32>,
}

impl ControlClient {
    pub async fn connect(cfg: &WorkerConfig) -> Result<Self> {
        let channel = Channel::from_shared(cfg.control_plane_addr.clone())
            .context("invalid control plane addr")?
            .connect()
            .await
            .context("connect to control plane")?;

        info!(addr = %cfg.control_plane_addr, worker_id = %cfg.worker_id, "connected to control plane");

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
            edge_worker_addr: cfg.edge_worker_addr.clone().unwrap_or_default(),
            nat_type: String::new(),
            active_calls: Arc::new(AtomicU32::new(0)),
        })
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
                labels: Default::default(),
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

fn cpu_usage_approx() -> f32 {
    use sysinfo::System;
    use std::sync::Mutex;
    static SYS: std::sync::OnceLock<Mutex<System>> = std::sync::OnceLock::new();
    let mut guard = SYS.get_or_init(|| Mutex::new(System::new())).lock().unwrap();
    guard.refresh_cpu_usage();
    guard.global_cpu_usage()
}
