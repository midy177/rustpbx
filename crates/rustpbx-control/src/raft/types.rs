//! Raft type configuration and the application command/response types.
//!
//! The replicated state is the worker registry: a high-churn, in-memory map of
//! live Media Workers that every control-plane replica must agree on. Persistent
//! config/CDR stays in the DB; only this ephemeral cluster state goes through Raft.

use openraft::BasicNode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Node id type — a simple monotonic integer assigned per control replica.
pub type NodeId = u64;

/// We need to track two addresses per control node:
/// - the **Raft transport** addr (where its `RaftService` gRPC server listens), and
/// - the **business gRPC** addr (its `ControlPlane` service), used to forward a
///   write to the leader when a follower receives it.
///
/// openraft's `BasicNode` carries a single `addr` string, so we pack both as
/// `"<raft_addr>|<grpc_addr>"`. Helpers below encode/decode; a value with no
/// `|` is treated as raft-only (grpc addr unknown), keeping older nodes working.
pub mod node_addr {
    use super::BasicNode;

    /// Build a `BasicNode` whose addr encodes both transport and business addrs.
    pub fn make(raft_addr: &str, grpc_addr: &str) -> BasicNode {
        BasicNode::new(format!("{raft_addr}|{grpc_addr}"))
    }

    /// The Raft transport address (for dialing the peer's `RaftService`).
    pub fn raft_addr(node_addr: &str) -> &str {
        node_addr.split('|').next().unwrap_or(node_addr)
    }

    /// The business gRPC address (`ControlPlane`), if encoded.
    pub fn grpc_addr(node_addr: &str) -> Option<&str> {
        node_addr.split('|').nth(1).filter(|s| !s.is_empty())
    }
}

/// A serializable snapshot of one registered Media Worker.
///
/// Mirrors `worker_registry::WorkerEntry` but uses wire-friendly types: times
/// are unix-millis (`i64`) instead of `tokio::time::Instant` (which is neither
/// serializable nor meaningful across processes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerRecord {
    pub worker_id: String,
    pub sip_addr: String,
    pub rtp_external_ip: String,
    pub rtp_start_port: u32,
    pub rtp_end_port: u32,
    pub max_concurrent: u32,
    pub active_calls: u32,
    pub cpu_usage: f32,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// EdgeWorker gRPC address (host:port) for AllocateCall; empty if absent.
    #[serde(default)]
    pub edge_worker_addr: String,
    /// Detected NAT type (STUN): open/cone/symmetric/nat/blocked/unknown.
    #[serde(default)]
    pub nat_type: String,
    /// Unix-millis when the worker first registered.
    pub registered_at_ms: i64,
    /// Unix-millis of the last heartbeat (or registration).
    pub last_heartbeat_ms: i64,
    pub draining: bool,
}

impl WorkerRecord {
    pub fn available_capacity(&self) -> u32 {
        self.max_concurrent.saturating_sub(self.active_calls)
    }

    pub fn tenant_affinity_score(&self, tenant_id: Option<i64>) -> u8 {
        let Some(tenant_id) = tenant_id else {
            return 0;
        };
        let tenant_id = tenant_id.to_string();
        let tenant_key = format!("tenant:{tenant_id}");
        (self.labels.get("tenant_id") == Some(&tenant_id)
            || self.labels.get("tenant") == Some(&tenant_id)
            || self
                .labels
                .get(&tenant_key)
                .is_some_and(|v| v == "true" || v == "1"))
        .into()
    }

    pub fn nat_reachability_score(&self) -> u8 {
        match self.nat_type.as_str() {
            "open" => 5,
            "cone" => 4,
            "nat" => 3,
            "" | "unknown" => 2,
            "symmetric" => 1,
            "blocked" => 0,
            _ => 2,
        }
    }

    /// Healthy = not draining and heartbeat within `timeout_ms` of `now_ms`.
    pub fn is_healthy(&self, now_ms: i64, timeout_ms: i64) -> bool {
        !self.draining && now_ms.saturating_sub(self.last_heartbeat_ms) < timeout_ms
    }
}

/// A serializable snapshot of one registered Edge gateway. Edges aren't
/// load-selected (so no capacity fields); this is purely for observability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeRecord {
    pub edge_id: String,
    pub public_ip: String,
    pub sip_addr: String,
    pub transports: Vec<String>,
    pub region: String,
    pub version: String,
    pub active_calls: u32,
    /// Detected NAT type (STUN): open/cone/symmetric/nat/blocked/unknown.
    #[serde(default)]
    pub nat_type: String,
    pub registered_at_ms: i64,
    pub last_heartbeat_ms: i64,
}

impl EdgeRecord {
    /// Healthy = heartbeat within `timeout_ms` of `now_ms`.
    pub fn is_healthy(&self, now_ms: i64, timeout_ms: i64) -> bool {
        now_ms.saturating_sub(self.last_heartbeat_ms) < timeout_ms
    }
}

/// A reserved per-tenant call slot, keyed by `call_id` in the state machine.
/// Tracked so concurrency enforcement is linearizable across control replicas
/// and survives leader changes (it rides the same Raft log as the registry).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallSlotRecord {
    pub tenant_id: i64,
    /// Optional source trunk name for trunk-level concurrency accounting.
    #[serde(default)]
    pub trunk_name: Option<String>,
    /// Unix-millis when the slot was reserved (for the TTL reaper backstop).
    pub at_ms: i64,
}

/// A recently accepted call start, used for trunk CPS enforcement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CallStartRecord {
    pub tenant_id: i64,
    #[serde(default)]
    pub trunk_name: Option<String>,
    pub at_ms: i64,
}

/// Commands applied to the replicated worker registry (the Raft `AppData`).
///
/// Every mutation of the registry goes through `Raft::client_write` with one of
/// these, so all replicas apply the same ordered log. Timestamps are supplied by
/// the leader at propose time (`*_ms` fields) so application is deterministic
/// across replicas (no replica reads its own clock during `apply`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RegistryCommand {
    /// Insert or replace a worker (register / re-register).
    Register { record: WorkerRecord },
    /// Update load fields + refresh the heartbeat timestamp.
    Heartbeat {
        worker_id: String,
        active_calls: u32,
        cpu_usage: f32,
        at_ms: i64,
    },
    /// Mark a worker as draining (stop routing new calls to it).
    Drain { worker_id: String },
    /// Remove a worker outright.
    Remove { worker_id: String },
    /// Remove every worker whose heartbeat is older than `before_ms`.
    ReapStale { before_ms: i64 },

    // ── Edge registry ──────────────────────────────────────────────────────────
    /// Insert or replace an edge (register / re-register).
    RegisterEdge { record: EdgeRecord },
    /// Refresh an edge's load + heartbeat timestamp.
    EdgeHeartbeat {
        edge_id: String,
        active_calls: u32,
        at_ms: i64,
    },
    /// Remove an edge outright.
    RemoveEdge { edge_id: String },
    /// Remove every edge whose heartbeat is older than `before_ms`.
    ReapStaleEdges { before_ms: i64 },

    // ── Per-tenant call slots (concurrency control) ─────────────────────────────
    /// Reserve a call slot for `tenant_id` keyed by `call_id`. If `max` is
    /// `Some(m)` and the tenant already holds ≥ m slots, the reservation is
    /// rejected (response `granted = false`). Re-acquiring an existing `call_id`
    /// is idempotent (always granted). Enforcement happens here, in log order,
    /// so two replicas can't both slip past the cap.
    AcquireCallSlot {
        call_id: String,
        tenant_id: i64,
        max: Option<u32>,
        trunk_name: Option<String>,
        trunk_max: Option<u32>,
        trunk_max_cps: Option<u32>,
        at_ms: i64,
    },
    /// Release the slot held by `call_id` (called when its CDR arrives).
    ReleaseCallSlot { call_id: String },
    /// Reap slots reserved before `before_ms` (crash/leak backstop).
    ReapCallSlots { before_ms: i64 },

    // ── Worker affinity (sticky routing) ─────────────────────────────────────
    /// Bind an affinity key (e.g. `conference:<tenant>:<room>`) to a worker.
    BindAffinity {
        affinity_key: String,
        worker_id: String,
        /// Optional unix-millis expiry. `None` means sticky until explicitly
        /// unbound or the selected worker is removed.
        #[serde(default)]
        expires_at_ms: Option<i64>,
    },
    /// Remove an affinity binding.
    UnbindAffinity { affinity_key: String },
    /// Remove one worker from an affinity binding.
    UnbindAffinityWorker {
        affinity_key: String,
        worker_id: String,
    },
    /// Remove affinity bindings whose expiry is at or before `before_ms`.
    ReapAffinity { before_ms: i64 },
}

// Note: `AppData` / `AppDataResponse` have blanket impls for any
// `Send + Sync + 'static` (serde) type, so we do NOT impl them manually.

/// Response from applying a `RegistryCommand`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegistryResponse {
    /// Whether the target worker was known at apply time (used by Heartbeat to
    /// tell an unknown worker to drain/re-register).
    pub known: bool,
    /// Number of entries removed (ReapStale / Remove / ReapCallSlots).
    pub removed: u32,
    /// AcquireCallSlot: whether the slot was granted. Defaulted so older
    /// serialized responses (no field) deserialize as granted.
    #[serde(default = "default_true")]
    pub granted: bool,
    /// AcquireCallSlot: the tenant's active-call count after applying.
    #[serde(default)]
    pub count: u32,
    /// AcquireCallSlot: source-trunk active-call count after applying.
    #[serde(default)]
    pub trunk_count: u32,
    /// AcquireCallSlot: source-trunk starts in the current 1s CPS window.
    #[serde(default)]
    pub trunk_cps_count: u32,
}

fn default_true() -> bool {
    true
}

impl RegistryResponse {
    /// The common worker/edge response (granted/count unused).
    pub fn known(known: bool, removed: u32) -> Self {
        Self {
            known,
            removed,
            granted: true,
            count: 0,
            trunk_count: 0,
            trunk_cps_count: 0,
        }
    }
}

openraft::declare_raft_types!(
    /// Raft type configuration for the control-plane worker registry.
    pub TypeConfig:
        D = RegistryCommand,
        R = RegistryResponse,
        NodeId = NodeId,
        Node = BasicNode,
        Entry = openraft::Entry<TypeConfig>,
        SnapshotData = std::io::Cursor<Vec<u8>>,
);
