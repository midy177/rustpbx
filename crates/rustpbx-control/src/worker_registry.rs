use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tracing::{info, warn};

/// A registered Media Worker.
#[derive(Debug, Clone)]
pub struct WorkerEntry {
    pub worker_id: String,
    /// Internal SIP address (Edge sends INVITE here)
    pub sip_addr: String,
    pub rtp_external_ip: String,
    pub rtp_start_port: u32,
    pub rtp_end_port: u32,
    pub max_concurrent: u32,
    pub active_calls: u32,
    pub cpu_usage: f32,
    pub registered_at: DateTime<Utc>,
    pub last_heartbeat: Instant,
    /// Whether this worker should stop accepting new calls
    pub draining: bool,
}

impl WorkerEntry {
    pub fn available_capacity(&self) -> u32 {
        self.max_concurrent.saturating_sub(self.active_calls)
    }

    pub fn is_healthy(&self, timeout: Duration) -> bool {
        !self.draining && self.last_heartbeat.elapsed() < timeout
    }
}

/// In-memory registry of live Media Workers.
///
/// Workers register on startup via gRPC RegisterWorker and send periodic
/// heartbeats.  Edge instances query GetAvailableWorkers to find a healthy
/// worker with spare capacity.
pub struct WorkerRegistry {
    workers: Arc<DashMap<String, WorkerEntry>>,
    heartbeat_timeout: Duration,
}

impl WorkerRegistry {
    pub fn new(heartbeat_timeout: Duration) -> Self {
        Self {
            workers: Arc::new(DashMap::new()),
            heartbeat_timeout,
        }
    }

    pub fn register(&self, entry: WorkerEntry) {
        info!(worker_id = %entry.worker_id, sip_addr = %entry.sip_addr, "worker registered");
        self.workers.insert(entry.worker_id.clone(), entry);
    }

    pub fn heartbeat(
        &self,
        worker_id: &str,
        active_calls: u32,
        cpu_usage: f32,
        rtp_ports_used: u32,
    ) -> bool {
        if let Some(mut entry) = self.workers.get_mut(worker_id) {
            entry.active_calls = active_calls;
            entry.cpu_usage = cpu_usage;
            entry.last_heartbeat = Instant::now();
            let _ = rtp_ports_used; // reserved for future metrics
            true
        } else {
            warn!(worker_id, "heartbeat from unknown worker");
            false
        }
    }

    pub fn drain(&self, worker_id: &str) {
        if let Some(mut entry) = self.workers.get_mut(worker_id) {
            entry.draining = true;
        }
    }

    /// Return healthy workers sorted by available capacity (most first).
    pub fn available(&self) -> Vec<WorkerEntry> {
        let timeout = self.heartbeat_timeout;
        let mut entries: Vec<WorkerEntry> = self
            .workers
            .iter()
            .filter(|e| e.is_healthy(timeout))
            .map(|e| e.clone())
            .collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.available_capacity()));
        entries
    }

    /// Select the least-loaded healthy worker for a new call.
    pub fn select_for_call(&self) -> Option<WorkerEntry> {
        self.available().into_iter().next()
    }

    pub fn remove(&self, worker_id: &str) {
        self.workers.remove(worker_id);
        info!(worker_id, "worker removed");
    }
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}
