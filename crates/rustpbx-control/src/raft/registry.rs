//! `RaftRegistry` — the worker registry backed by single-node Raft.
//!
//! Drop-in replacement for the old in-memory `WorkerRegistry`: every mutation
//! goes through `Raft::client_write` (so it's replicated once multiple replicas
//! join), and reads are served from the local state machine. In single-node
//! mode this node is the only voter, so writes commit immediately.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::Raft;
use tracing::info;

use super::log_store::LogStore;
use super::network::NetworkFactory;
use super::state_machine::StateMachineStore;
use super::types::{NodeId, RegistryCommand, TypeConfig, WorkerRecord};

/// Current wall-clock in unix-millis. Used by the leader to stamp commands so
/// state-machine application stays deterministic across replicas.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone)]
pub struct RaftRegistry {
    raft: Raft<TypeConfig>,
    sm: StateMachineStore,
    heartbeat_timeout: Duration,
}

impl RaftRegistry {
    /// Start a single-node Raft and initialize this node as the sole voter.
    pub async fn start(node_id: NodeId, heartbeat_timeout: Duration) -> Result<Self> {
        let config = Config {
            cluster_name: "rustpbx-control".to_string(),
            // Snapshot fairly often: the registry is small and this keeps the
            // in-memory log from growing unbounded.
            snapshot_policy: openraft::SnapshotPolicy::LogsSinceLast(1024),
            ..Default::default()
        };
        let config = Arc::new(config.validate()?);

        let log_store = LogStore::new();
        let sm = StateMachineStore::new();
        let network = NetworkFactory;

        let raft = Raft::new(node_id, config, network, log_store, sm.clone()).await?;

        // Initialize a single-voter cluster (this node only). Idempotent-ish:
        // if already initialized (e.g. restart with persisted log — not in
        // Phase 1) this errors, which we tolerate.
        let mut members = BTreeMap::new();
        members.insert(node_id, openraft::BasicNode::new("127.0.0.1:0"));
        match raft.initialize(members).await {
            Ok(()) => info!(node_id, "raft initialized as single-voter cluster"),
            Err(e) => info!(node_id, error = %e, "raft already initialized or init skipped"),
        }

        Ok(Self {
            raft,
            sm,
            heartbeat_timeout,
        })
    }

    pub fn heartbeat_timeout(&self) -> Duration {
        self.heartbeat_timeout
    }

    /// The underlying Raft handle (for metrics / graceful shutdown / membership
    /// changes — wired in the multi-replica phase).
    #[allow(dead_code)]
    pub fn raft(&self) -> &Raft<TypeConfig> {
        &self.raft
    }

    /// Register (or replace) a worker. `registered_at_ms` / `last_heartbeat_ms`
    /// are stamped here if zero.
    pub async fn register(&self, mut record: WorkerRecord) -> Result<()> {
        let now = now_ms();
        if record.registered_at_ms == 0 {
            record.registered_at_ms = now;
        }
        record.last_heartbeat_ms = now;
        self.raft
            .client_write(RegistryCommand::Register { record })
            .await?;
        Ok(())
    }

    /// Record a heartbeat. Returns whether the worker was known.
    pub async fn heartbeat(
        &self,
        worker_id: &str,
        active_calls: u32,
        cpu_usage: f32,
    ) -> Result<bool> {
        let resp = self
            .raft
            .client_write(RegistryCommand::Heartbeat {
                worker_id: worker_id.to_string(),
                active_calls,
                cpu_usage,
                at_ms: now_ms(),
            })
            .await?;
        Ok(resp.data.known)
    }

    /// Mark a worker as draining.
    #[allow(dead_code)]
    pub async fn drain(&self, worker_id: &str) -> Result<()> {
        self.raft
            .client_write(RegistryCommand::Drain {
                worker_id: worker_id.to_string(),
            })
            .await?;
        Ok(())
    }

    /// Remove a worker outright.
    #[allow(dead_code)]
    pub async fn remove(&self, worker_id: &str) -> Result<()> {
        self.raft
            .client_write(RegistryCommand::Remove {
                worker_id: worker_id.to_string(),
            })
            .await?;
        Ok(())
    }

    /// Remove workers whose heartbeat is stale past 2× the timeout. Returns the
    /// number reaped.
    pub async fn reap_stale(&self) -> Result<u32> {
        let threshold_ms = (self.heartbeat_timeout.as_millis() as i64) * 2;
        let before_ms = now_ms() - threshold_ms;
        let resp = self
            .raft
            .client_write(RegistryCommand::ReapStale { before_ms })
            .await?;
        Ok(resp.data.removed)
    }

    /// All registered workers (healthy or not) — for the admin API.
    pub async fn all(&self) -> Vec<WorkerRecord> {
        self.sm.list_workers().await
    }

    /// Healthy workers with spare capacity, most-available first.
    pub async fn available(&self) -> Vec<WorkerRecord> {
        let now = now_ms();
        let timeout_ms = self.heartbeat_timeout.as_millis() as i64;
        let mut workers: Vec<WorkerRecord> = self
            .sm
            .list_workers()
            .await
            .into_iter()
            .filter(|w| w.is_healthy(now, timeout_ms))
            .collect();
        workers.sort_by_key(|w| std::cmp::Reverse(w.available_capacity()));
        workers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str) -> WorkerRecord {
        WorkerRecord {
            worker_id: id.to_string(),
            sip_addr: "127.0.0.1:5060".to_string(),
            rtp_external_ip: "127.0.0.1".to_string(),
            rtp_start_port: 10000,
            rtp_end_port: 20000,
            max_concurrent: 100,
            active_calls: 0,
            cpu_usage: 0.0,
            registered_at_ms: 0,
            last_heartbeat_ms: 0,
            draining: false,
        }
    }

    async fn start() -> RaftRegistry {
        RaftRegistry::start(1, Duration::from_secs(30)).await.unwrap()
    }

    #[tokio::test]
    async fn register_then_read_back_through_raft() {
        let reg = start().await;
        reg.register(rec("w1")).await.unwrap();
        reg.register(rec("w2")).await.unwrap();

        let all = reg.all().await;
        assert_eq!(all.len(), 2, "both workers should be committed and readable");

        let avail = reg.available().await;
        assert_eq!(avail.len(), 2, "fresh workers are healthy and available");
    }

    #[tokio::test]
    async fn heartbeat_updates_load_and_reports_known() {
        let reg = start().await;
        reg.register(rec("w1")).await.unwrap();

        let known = reg.heartbeat("w1", 7, 0.5).await.unwrap();
        assert!(known, "heartbeat for a registered worker is known");

        let unknown = reg.heartbeat("ghost", 1, 0.1).await.unwrap();
        assert!(!unknown, "heartbeat for an unregistered worker is unknown");

        let w = reg.all().await.into_iter().find(|w| w.worker_id == "w1").unwrap();
        assert_eq!(w.active_calls, 7, "heartbeat updates active_calls");
    }

    #[tokio::test]
    async fn reap_removes_only_stale_workers() {
        let reg = start().await;
        // Fresh worker: heartbeat stamped to now by register().
        reg.register(rec("fresh")).await.unwrap();

        // Stale worker: insert directly with an ancient heartbeat via a manual
        // command (bypass register()'s now-stamping).
        let mut old = rec("stale");
        old.registered_at_ms = 1;
        old.last_heartbeat_ms = 1; // 1970 — definitely older than 2× timeout
        reg.raft()
            .client_write(RegistryCommand::Register { record: old })
            .await
            .unwrap();
        // register() above re-stamps last_heartbeat, so push the stale ts again.
        reg.raft()
            .client_write(RegistryCommand::Heartbeat {
                worker_id: "stale".to_string(),
                active_calls: 0,
                cpu_usage: 0.0,
                at_ms: 1,
            })
            .await
            .unwrap();

        let removed = reg.reap_stale().await.unwrap();
        assert_eq!(removed, 1, "only the stale worker is reaped");

        let ids: Vec<String> = reg.all().await.into_iter().map(|w| w.worker_id).collect();
        assert_eq!(ids, vec!["fresh".to_string()]);
    }
}
