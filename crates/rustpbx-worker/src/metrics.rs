/// Prometheus metrics for the Worker.
///
/// Exposes:
///   worker_active_calls       — current concurrent calls
///   worker_total_calls        — lifetime call counter
///   worker_rtp_ports_used     — RTP ports in use
use metrics::{counter, gauge};
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

pub struct WorkerMetrics {
    pub active_calls: Arc<AtomicU32>,
    pub total_calls: Arc<AtomicU64>,
    pub rtp_ports_used: Arc<AtomicU32>,
}

impl WorkerMetrics {
    pub fn new() -> Self {
        Self {
            active_calls: Arc::new(AtomicU32::new(0)),
            total_calls: Arc::new(AtomicU64::new(0)),
            rtp_ports_used: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn call_started(&self) {
        self.active_calls.fetch_add(1, Ordering::Relaxed);
        self.total_calls.fetch_add(1, Ordering::Relaxed);
        gauge!("worker_active_calls").increment(1.0);
        counter!("worker_total_calls").increment(1);
    }

    pub fn call_ended(&self) {
        self.active_calls.fetch_sub(1, Ordering::Relaxed);
        gauge!("worker_active_calls").decrement(1.0);
    }

    pub fn active(&self) -> u32 {
        self.active_calls.load(Ordering::Relaxed)
    }
}

impl Default for WorkerMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Start the Prometheus metrics HTTP server.
///
/// Buckets are tuned for telephony workloads (matching the observability addon)
/// so that `rustpbx_call_duration_seconds`, `rustpbx_invite_latency_seconds`,
/// etc. produce meaningful histogram percentiles.
pub async fn start_metrics_server(addr: &str) -> anyhow::Result<()> {
    use metrics_exporter_prometheus::PrometheusBuilder;

    let addr: std::net::SocketAddr = addr.parse()?;
    PrometheusBuilder::new()
        .set_buckets(&[
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
        ])
        .map_err(|e| anyhow::anyhow!("failed to configure Prometheus buckets: {e}"))?
        .with_http_listener(addr)
        .install()?;
    tracing::info!(%addr, "prometheus metrics listening");
    Ok(())
}
