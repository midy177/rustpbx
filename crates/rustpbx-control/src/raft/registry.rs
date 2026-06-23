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
use serde::Serialize;
use tracing::info;

use super::log_store::LogStore;
use super::network::NetworkFactory;
use super::state_machine::StateMachineStore;
use super::types::{
    node_addr, EdgeRecord, NodeId, RegistryCommand, RegistryResponse, TypeConfig, WorkerRecord,
};

/// Current wall-clock in unix-millis. Used by the leader to stamp commands so
/// state-machine application stays deterministic across replicas.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Compact view of Raft cluster state for the admin API.
#[derive(Debug, Clone, Serialize)]
pub struct RaftMetricsSummary {
    pub id: NodeId,
    pub state: String,
    pub current_term: u64,
    pub current_leader: Option<NodeId>,
    pub last_log_index: Option<u64>,
    pub last_applied: Option<u64>,
    /// node_id -> advertised address.
    pub members: Vec<(NodeId, String)>,
}

#[derive(Clone)]
pub struct RaftRegistry {
    raft: Raft<TypeConfig>,
    sm: StateMachineStore,
    heartbeat_timeout: Duration,
    /// Client TLS used when forwarding writes to the leader's business gRPC.
    tls: Option<tonic::transport::ClientTlsConfig>,
}

impl RaftRegistry {
    /// Build the Raft node without initializing any cluster membership. Used by
    /// both single-node and cluster startup paths. `tls` is applied to both the
    /// Raft transport client and the leader-forwarding client.
    async fn build(
        node_id: NodeId,
        heartbeat_timeout: Duration,
        tls: Option<tonic::transport::ClientTlsConfig>,
    ) -> Result<Self> {
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
        let network = NetworkFactory::new(tls.clone());

        let raft = Raft::new(node_id, config, network, log_store, sm.clone()).await?;

        Ok(Self {
            raft,
            sm,
            heartbeat_timeout,
            tls,
        })
    }

    /// Start a single-node Raft and initialize this node as the sole voter.
    /// Backward-compatible default when no Raft address is configured.
    pub async fn start(node_id: NodeId, heartbeat_timeout: Duration) -> Result<Self> {
        let this = Self::build(node_id, heartbeat_timeout, None).await?;

        // Initialize a single-voter cluster (this node only).
        let mut members = BTreeMap::new();
        members.insert(node_id, openraft::BasicNode::new("127.0.0.1:0"));
        match this.raft.initialize(members).await {
            Ok(()) => info!(node_id, "raft initialized as single-voter cluster"),
            Err(e) => info!(node_id, error = %e, "raft already initialized or init skipped"),
        }

        Ok(this)
    }

    /// Start a cluster-mode Raft node advertising both its Raft transport addr
    /// (`advertise_addr`) and its business gRPC addr (`grpc_addr`) to peers.
    ///
    /// If `bootstrap` is true, this node initializes a fresh single-voter
    /// cluster that other nodes then join via `add_learner` + `change_membership`.
    /// If false, the node starts uninitialized and waits to be added by a leader.
    pub async fn start_cluster(
        node_id: NodeId,
        advertise_addr: &str,
        grpc_addr: &str,
        bootstrap: bool,
        heartbeat_timeout: Duration,
        tls: Option<tonic::transport::ClientTlsConfig>,
    ) -> Result<Self> {
        let this = Self::build(node_id, heartbeat_timeout, tls).await?;

        if bootstrap && !this.raft.is_initialized().await? {
            let mut members = BTreeMap::new();
            members.insert(node_id, node_addr::make(advertise_addr, grpc_addr));
            match this.raft.initialize(members).await {
                Ok(()) => info!(node_id, advertise_addr, grpc_addr, "raft cluster bootstrapped (single voter)"),
                Err(e) => info!(node_id, error = %e, "raft bootstrap skipped"),
            }
        } else {
            info!(
                node_id,
                advertise_addr, "raft node started uninitialized; awaiting join from a cluster leader"
            );
        }

        Ok(this)
    }

    pub fn heartbeat_timeout(&self) -> Duration {
        self.heartbeat_timeout
    }

    /// The underlying Raft handle (for graceful shutdown / advanced ops).
    #[allow(dead_code)]
    pub fn raft(&self) -> &Raft<TypeConfig> {
        &self.raft
    }

    /// Add a node as a learner (non-voting replica that receives the log).
    /// `raft_addr` is the new node's Raft transport addr; `grpc_addr` its
    /// business gRPC addr (used for write-forwarding). First step of joining a
    /// new replica; follow with `change_membership` to promote it to a voter.
    pub async fn add_learner(&self, node_id: NodeId, raft_addr: &str, grpc_addr: &str) -> Result<()> {
        self.raft
            .add_learner(node_id, node_addr::make(raft_addr, grpc_addr), true)
            .await?;
        Ok(())
    }

    /// Set the cluster's voter membership to exactly `voters`. Learners not in
    /// the set are retained as learners (`retain = true`).
    pub async fn change_membership(&self, voters: std::collections::BTreeSet<NodeId>) -> Result<()> {
        self.raft.change_membership(voters, true).await?;
        Ok(())
    }

    /// A compact, JSON-friendly snapshot of Raft state for the admin API.
    pub fn metrics_summary(&self) -> RaftMetricsSummary {
        let m = self.raft.metrics().borrow().clone();
        RaftMetricsSummary {
            id: m.id,
            state: format!("{:?}", m.state),
            current_term: m.current_term,
            current_leader: m.current_leader,
            last_log_index: m.last_log_index,
            last_applied: m.last_applied.map(|l| l.index),
            members: m
                .membership_config
                .nodes()
                .map(|(id, node)| (*id, node.addr.clone()))
                .collect(),
        }
    }

    /// Propose a registry mutation through Raft. If this node is not the leader,
    /// openraft returns `ForwardToLeader`; we then forward the serialized command
    /// to the leader's `ProposeWrite` gRPC so the write still lands. This makes a
    /// worker's register/heartbeat work no matter which replica it hit.
    async fn propose(&self, cmd: RegistryCommand) -> Result<RegistryResponse> {
        match self.raft.client_write(cmd.clone()).await {
            Ok(resp) => Ok(resp.data),
            Err(e) => {
                // Only ForwardToLeader is forwardable; anything else propagates.
                let fwd = e.forward_to_leader::<openraft::BasicNode>().cloned();
                match fwd {
                    Some(f) => self.forward_to_leader(cmd, f).await,
                    None => Err(e.into()),
                }
            }
        }
    }

    /// Forward a command to the current leader's business gRPC (`ProposeWrite`).
    async fn forward_to_leader(
        &self,
        cmd: RegistryCommand,
        fwd: openraft::error::ForwardToLeader<NodeId, openraft::BasicNode>,
    ) -> Result<RegistryResponse> {
        let leader = fwd
            .leader_node
            .as_ref()
            .and_then(|n| node_addr::grpc_addr(&n.addr).map(|s| s.to_string()))
            .ok_or_else(|| anyhow::anyhow!("no known leader to forward write to"))?;

        let scheme = if self.tls.is_some() { "https" } else { "http" };
        let endpoint_url = if leader.starts_with("http") {
            leader
        } else {
            format!("{scheme}://{leader}")
        };
        let mut endpoint = tonic::transport::Endpoint::from_shared(endpoint_url)?;
        if let Some(tls) = &self.tls {
            endpoint = endpoint.tls_config(tls.clone())?;
        }
        let channel = endpoint.connect().await?;
        let command = serde_json::to_vec(&cmd)?;
        let mut client =
            crate::grpc::proto::control::control_plane_client::ControlPlaneClient::new(channel);
        let resp = client
            .propose_write(crate::grpc::proto::control::ProposeWriteRequest { command })
            .await?
            .into_inner();
        Ok(RegistryResponse {
            known: resp.known,
            removed: resp.removed,
        })
    }

    /// Apply a command that was forwarded to us as the leader. Called by the
    /// `ProposeWrite` gRPC handler.
    pub async fn apply_forwarded(&self, cmd: RegistryCommand) -> Result<RegistryResponse> {
        // We're presumably the leader; if not, this re-forwards (rare race).
        self.propose(cmd).await
    }

    /// Register (or replace) a worker. `registered_at_ms` / `last_heartbeat_ms`
    /// are stamped here if zero.
    pub async fn register(&self, mut record: WorkerRecord) -> Result<()> {
        let now = now_ms();
        if record.registered_at_ms == 0 {
            record.registered_at_ms = now;
        }
        record.last_heartbeat_ms = now;
        self.propose(RegistryCommand::Register { record }).await?;
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
            .propose(RegistryCommand::Heartbeat {
                worker_id: worker_id.to_string(),
                active_calls,
                cpu_usage,
                at_ms: now_ms(),
            })
            .await?;
        Ok(resp.known)
    }

    /// Mark a worker as draining.
    #[allow(dead_code)]
    pub async fn drain(&self, worker_id: &str) -> Result<()> {
        self.propose(RegistryCommand::Drain {
            worker_id: worker_id.to_string(),
        })
        .await?;
        Ok(())
    }

    /// Remove a worker outright.
    #[allow(dead_code)]
    pub async fn remove(&self, worker_id: &str) -> Result<()> {
        self.propose(RegistryCommand::Remove {
            worker_id: worker_id.to_string(),
        })
        .await?;
        Ok(())
    }

    /// Remove workers whose heartbeat is stale past 2× the timeout. Returns the
    /// number reaped. Reaping is best-effort on followers — only the leader's
    /// reaper actually commits (a follower's propose forwards to the leader).
    pub async fn reap_stale(&self) -> Result<u32> {
        let threshold_ms = (self.heartbeat_timeout.as_millis() as i64) * 2;
        let before_ms = now_ms() - threshold_ms;
        let resp = self.propose(RegistryCommand::ReapStale { before_ms }).await?;
        Ok(resp.removed)
    }

    /// All registered workers (healthy or not) — for the admin API.
    pub async fn all(&self) -> Vec<WorkerRecord> {
        self.sm.list_workers().await
    }

    // ── Edge registry ──────────────────────────────────────────────────────────

    /// Register (or replace) an edge. Timestamps are stamped here if zero.
    pub async fn register_edge(&self, mut record: EdgeRecord) -> Result<()> {
        let now = now_ms();
        if record.registered_at_ms == 0 {
            record.registered_at_ms = now;
        }
        record.last_heartbeat_ms = now;
        self.propose(RegistryCommand::RegisterEdge { record }).await?;
        Ok(())
    }

    /// Record an edge heartbeat. Returns whether the edge was known.
    pub async fn edge_heartbeat(&self, edge_id: &str, active_calls: u32) -> Result<bool> {
        let resp = self
            .propose(RegistryCommand::EdgeHeartbeat {
                edge_id: edge_id.to_string(),
                active_calls,
                at_ms: now_ms(),
            })
            .await?;
        Ok(resp.known)
    }

    /// Remove edges whose heartbeat is stale past 2× the timeout. Returns the
    /// number reaped (leader commits; followers forward).
    pub async fn reap_stale_edges(&self) -> Result<u32> {
        let threshold_ms = (self.heartbeat_timeout.as_millis() as i64) * 2;
        let before_ms = now_ms() - threshold_ms;
        let resp = self.propose(RegistryCommand::ReapStaleEdges { before_ms }).await?;
        Ok(resp.removed)
    }

    /// All registered edges (healthy or not) — for the admin API.
    pub async fn all_edges(&self) -> Vec<EdgeRecord> {
        self.sm.list_edges().await
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
            edge_worker_addr: String::new(),
            registered_at_ms: 0,
            last_heartbeat_ms: 0,
            draining: false,
        }
    }

    fn edge_rec(id: &str) -> EdgeRecord {
        EdgeRecord {
            edge_id: id.to_string(),
            public_ip: "203.0.113.7".to_string(),
            sip_addr: "203.0.113.7:5060".to_string(),
            transports: vec!["udp".to_string()],
            region: "us-east".to_string(),
            version: "0.1.0".to_string(),
            active_calls: 0,
            registered_at_ms: 0,
            last_heartbeat_ms: 0,
        }
    }

    async fn start() -> RaftRegistry {
        RaftRegistry::start(1, Duration::from_secs(30)).await.unwrap()
    }

    #[tokio::test]
    async fn edge_register_heartbeat_and_reap() {
        let reg = start().await;
        reg.register_edge(edge_rec("edge-1")).await.unwrap();
        reg.register_edge(edge_rec("edge-2")).await.unwrap();
        assert_eq!(reg.all_edges().await.len(), 2, "both edges committed");

        // Heartbeat updates load and is known; unknown edge reports not-known.
        assert!(reg.edge_heartbeat("edge-1", 5).await.unwrap());
        assert!(!reg.edge_heartbeat("ghost", 1).await.unwrap());
        let e1 = reg.all_edges().await.into_iter().find(|e| e.edge_id == "edge-1").unwrap();
        assert_eq!(e1.active_calls, 5);

        // Force edge-2 stale, then reap.
        reg.raft()
            .client_write(RegistryCommand::EdgeHeartbeat {
                edge_id: "edge-2".to_string(),
                active_calls: 0,
                at_ms: 1,
            })
            .await
            .unwrap();
        let removed = reg.reap_stale_edges().await.unwrap();
        assert_eq!(removed, 1, "only the stale edge is reaped");
        let ids: Vec<String> = reg.all_edges().await.into_iter().map(|e| e.edge_id).collect();
        assert_eq!(ids, vec!["edge-1".to_string()]);
        // Worker registry is unaffected by edge commands.
        assert!(reg.all().await.is_empty());
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

    /// Start a node's Raft gRPC transport on `addr` so peers can reach it.
    fn spawn_raft_server(reg: &RaftRegistry, addr: std::net::SocketAddr) {
        use crate::grpc::proto::raft::raft_service_server::RaftServiceServer;
        let server = crate::raft::server::RaftServer::new(reg.raft().clone());
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(RaftServiceServer::new(server))
                .serve(addr)
                .await
                .unwrap();
        });
    }

    /// Full two-node cluster: bootstrap node 1, start node 2 uninitialized,
    /// join it as a learner then promote to voter, write on the leader, and
    /// verify the write replicates to the follower's state machine. Exercises
    /// the real gRPC transport, serialization, leader election and replication.
    #[tokio::test]
    async fn two_node_cluster_replicates_writes() {
        let addr1: std::net::SocketAddr = "127.0.0.1:24101".parse().unwrap();
        let addr2: std::net::SocketAddr = "127.0.0.1:24102".parse().unwrap();
        let hb = Duration::from_secs(30);

        // Node 1 bootstraps a single-voter cluster advertising its real addr.
        // (grpc_addr unused here — this test writes only on the leader.)
        let n1 = RaftRegistry::start_cluster(1, &addr1.to_string(), &addr1.to_string(), true, hb, None)
            .await
            .unwrap();
        // Node 2 starts uninitialized, waiting to be added.
        let n2 = RaftRegistry::start_cluster(2, &addr2.to_string(), &addr2.to_string(), false, hb, None)
            .await
            .unwrap();

        spawn_raft_server(&n1, addr1);
        spawn_raft_server(&n2, addr2);

        // Let node 1 win the initial election.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Join node 2: learner first, then promote both to voters.
        n1.add_learner(2, &addr2.to_string(), &addr2.to_string()).await.unwrap();
        n1.change_membership([1, 2].into_iter().collect())
            .await
            .unwrap();

        // Write on the leader (node 1).
        n1.register(rec("replicated-worker")).await.unwrap();

        // Wait for replication, then read node 2's local state machine directly.
        let mut found = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if n2.all().await.iter().any(|w| w.worker_id == "replicated-worker") {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "write on the leader must replicate to the follower's state machine"
        );

        // Sanity: node 2 sees node 1 as leader and 2 voters in the membership.
        let m2 = n2.metrics_summary();
        assert_eq!(m2.current_leader, Some(1), "node 2 should follow leader 1");
        assert_eq!(m2.members.len(), 2, "membership should contain both nodes");
    }

    /// Start a node's business ControlPlane gRPC server (for write-forwarding).
    /// Uses an in-memory DB since the forwarding path only touches the registry.
    async fn spawn_control_server(reg: &RaftRegistry, addr: std::net::SocketAddr) {
        use crate::grpc::proto::control::control_plane_server::ControlPlaneServer;
        use crate::store::Store;
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        let svc = crate::grpc::control_plane::ControlPlaneService::new(
            std::sync::Arc::new(Store::new(db)),
            reg.clone(),
        );
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(ControlPlaneServer::new(svc))
                .serve(addr)
                .await
                .unwrap();
        });
    }

    /// Write hits the FOLLOWER: `register` on node 2 must forward to leader
    /// node 1 over the business gRPC, commit there, and replicate back to
    /// node 2's state machine. Exercises the full leader-forwarding path.
    #[tokio::test]
    async fn follower_forwards_write_to_leader() {
        let raft1: std::net::SocketAddr = "127.0.0.1:24111".parse().unwrap();
        let raft2: std::net::SocketAddr = "127.0.0.1:24112".parse().unwrap();
        let grpc1: std::net::SocketAddr = "127.0.0.1:24113".parse().unwrap();
        let grpc2: std::net::SocketAddr = "127.0.0.1:24114".parse().unwrap();
        let hb = Duration::from_secs(30);

        // Each node advertises its raft addr + its business grpc addr.
        let n1 = RaftRegistry::start_cluster(1, &raft1.to_string(), &grpc1.to_string(), true, hb, None)
            .await
            .unwrap();
        let n2 = RaftRegistry::start_cluster(2, &raft2.to_string(), &grpc2.to_string(), false, hb, None)
            .await
            .unwrap();

        spawn_raft_server(&n1, raft1);
        spawn_raft_server(&n2, raft2);
        spawn_control_server(&n1, grpc1).await;
        spawn_control_server(&n2, grpc2).await;

        tokio::time::sleep(Duration::from_millis(500)).await;
        n1.add_learner(2, &raft2.to_string(), &grpc2.to_string()).await.unwrap();
        n1.change_membership([1, 2].into_iter().collect()).await.unwrap();

        // Write on the FOLLOWER (node 2). Must succeed via forwarding.
        n2.register(rec("forwarded-worker")).await.unwrap();

        // It should be visible on both nodes' state machines.
        let mut on_leader = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if n1.all().await.iter().any(|w| w.worker_id == "forwarded-worker") {
                on_leader = true;
                break;
            }
        }
        assert!(on_leader, "forwarded write must commit on the leader");
        assert!(
            n2.all().await.iter().any(|w| w.worker_id == "forwarded-worker"),
            "and replicate back to the follower"
        );
    }

    /// Start a Raft transport server with TLS.
    fn spawn_raft_server_tls(
        reg: &RaftRegistry,
        addr: std::net::SocketAddr,
        server_tls: tonic::transport::ServerTlsConfig,
    ) {
        use crate::grpc::proto::raft::raft_service_server::RaftServiceServer;
        let server = crate::raft::server::RaftServer::new(reg.raft().clone());
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .tls_config(server_tls)
                .unwrap()
                .add_service(RaftServiceServer::new(server))
                .serve(addr)
                .await
                .unwrap();
        });
    }

    /// Two-node cluster over **TLS**: generate a self-signed cert for
    /// "localhost", run both Raft transports with server TLS, dial peers with
    /// client TLS (CA = the self-signed cert), and verify replication. Proves
    /// the TLS plumbing (server identity + client CA verification + https
    /// scheme) actually works, not just compiles.
    #[tokio::test]
    async fn two_node_cluster_over_tls_replicates() {
        // Self-signed cert valid for "localhost" (the SocketAddrs use 127.0.0.1,
        // but we set the client domain_name to "localhost" to match the SAN).
        let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_pem = ck.cert.pem();
        let key_pem = ck.signing_key.serialize_pem();

        let server_tls = || {
            tonic::transport::ServerTlsConfig::new().identity(
                tonic::transport::Identity::from_pem(cert_pem.clone(), key_pem.clone()),
            )
        };
        let client_tls = tonic::transport::ClientTlsConfig::new()
            .ca_certificate(tonic::transport::Certificate::from_pem(cert_pem.clone()))
            .domain_name("localhost");

        let addr1: std::net::SocketAddr = "127.0.0.1:24121".parse().unwrap();
        let addr2: std::net::SocketAddr = "127.0.0.1:24122".parse().unwrap();
        // Peers must be reached by the cert's SAN name → advertise "localhost:port".
        let adv1 = format!("localhost:{}", addr1.port());
        let adv2 = format!("localhost:{}", addr2.port());
        let hb = Duration::from_secs(30);

        let n1 = RaftRegistry::start_cluster(1, &adv1, &adv1, true, hb, Some(client_tls.clone()))
            .await
            .unwrap();
        let n2 = RaftRegistry::start_cluster(2, &adv2, &adv2, false, hb, Some(client_tls.clone()))
            .await
            .unwrap();

        spawn_raft_server_tls(&n1, addr1, server_tls());
        spawn_raft_server_tls(&n2, addr2, server_tls());

        tokio::time::sleep(Duration::from_millis(500)).await;
        n1.add_learner(2, &adv2, &adv2).await.unwrap();
        n1.change_membership([1, 2].into_iter().collect()).await.unwrap();

        n1.register(rec("tls-worker")).await.unwrap();

        let mut found = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if n2.all().await.iter().any(|w| w.worker_id == "tls-worker") {
                found = true;
                break;
            }
        }
        assert!(found, "write must replicate to the follower over TLS");
        assert_eq!(n2.metrics_summary().current_leader, Some(1));
    }
}
