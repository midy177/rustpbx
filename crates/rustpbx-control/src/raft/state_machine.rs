//! Raft state machine: the replicated worker registry.
//!
//! Applies [`RegistryCommand`]s in log order to an in-memory map of
//! [`WorkerRecord`]s. Snapshots are a serialized copy of the whole map plus the
//! last-applied log id and membership — small and cheap, since the registry is
//! at most a few hundred workers.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use openraft::storage::RaftSnapshotBuilder;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use openraft::LogId;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::StoredMembership;
use openraft::EntryPayload;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::types::{
    CallSlotRecord, EdgeRecord, NodeId, RegistryCommand, RegistryResponse, TypeConfig, WorkerRecord,
};

/// The data captured in a snapshot: the full state machine contents.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StoredSnapshot {
    pub last_applied: Option<LogId<NodeId>>,
    pub last_membership: StoredMembership<NodeId, openraft::BasicNode>,
    /// Serialized worker map.
    pub workers: BTreeMap<String, WorkerRecord>,
    /// Serialized edge map (defaulted for snapshots written before edges existed).
    #[serde(default)]
    pub edges: BTreeMap<String, EdgeRecord>,
    /// Per-tenant call slots keyed by call_id (defaulted for older snapshots).
    #[serde(default)]
    pub call_slots: BTreeMap<String, CallSlotRecord>,
}

/// In-memory state machine data, guarded for shared async access.
#[derive(Debug, Default)]
struct StateMachineData {
    last_applied: Option<LogId<NodeId>>,
    last_membership: StoredMembership<NodeId, openraft::BasicNode>,
    /// worker_id -> record
    workers: BTreeMap<String, WorkerRecord>,
    /// edge_id -> record
    edges: BTreeMap<String, EdgeRecord>,
    /// call_id -> reserved call slot (per-tenant concurrency control)
    call_slots: BTreeMap<String, CallSlotRecord>,
}

/// The state machine store: shared handle around the data plus the latest
/// built snapshot. Cloneable so the snapshot builder can share the same data.
#[derive(Clone, Debug, Default)]
pub struct StateMachineStore {
    data: Arc<Mutex<StateMachineData>>,
    /// The most recently built/installed snapshot (for `get_current_snapshot`).
    current_snapshot: Arc<Mutex<Option<Snapshot<TypeConfig>>>>,
    /// Monotonic counter to disambiguate snapshot ids built at the same log id.
    snapshot_idx: Arc<Mutex<u64>>,
}

impl StateMachineStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read-only snapshot of the current worker map (for serving queries
    /// locally without going through Raft).
    pub async fn list_workers(&self) -> Vec<WorkerRecord> {
        self.data.lock().await.workers.values().cloned().collect()
    }

    /// Read-only snapshot of the current edge map.
    pub async fn list_edges(&self) -> Vec<EdgeRecord> {
        self.data.lock().await.edges.values().cloned().collect()
    }

    /// Current number of reserved call slots for a tenant (read-only).
    pub async fn tenant_call_count(&self, tenant_id: i64) -> u32 {
        self.data
            .lock()
            .await
            .call_slots
            .values()
            .filter(|s| s.tenant_id == tenant_id)
            .count() as u32
    }

    /// Total reserved call slots across all tenants (read-only, for stats).
    pub async fn call_slot_count(&self) -> u32 {
        self.data.lock().await.call_slots.len() as u32
    }
}

/// Count a tenant's currently-held slots in the map.
fn tenant_slots(slots: &BTreeMap<String, CallSlotRecord>, tenant_id: i64) -> u32 {
    slots.values().filter(|s| s.tenant_id == tenant_id).count() as u32
}

/// Apply one command to the registry maps. Pure function of (data, command) so
/// the result is identical on every replica.
fn apply_command(data: &mut StateMachineData, cmd: RegistryCommand) -> RegistryResponse {
    let workers = &mut data.workers;
    match cmd {
        RegistryCommand::Register { record } => {
            workers.insert(record.worker_id.clone(), record);
            RegistryResponse::known(true, 0)
        }
        RegistryCommand::Heartbeat { worker_id, active_calls, cpu_usage, at_ms } => {
            if let Some(w) = workers.get_mut(&worker_id) {
                w.active_calls = active_calls;
                w.cpu_usage = cpu_usage;
                w.last_heartbeat_ms = at_ms;
                RegistryResponse::known(true, 0)
            } else {
                RegistryResponse::known(false, 0)
            }
        }
        RegistryCommand::Drain { worker_id } => {
            let known = if let Some(w) = workers.get_mut(&worker_id) {
                w.draining = true;
                true
            } else {
                false
            };
            RegistryResponse::known(known, 0)
        }
        RegistryCommand::Remove { worker_id } => {
            let removed = workers.remove(&worker_id).is_some() as u32;
            RegistryResponse::known(removed > 0, removed)
        }
        RegistryCommand::ReapStale { before_ms } => {
            let before = workers.len();
            workers.retain(|_, w| w.last_heartbeat_ms >= before_ms);
            let removed = (before - workers.len()) as u32;
            RegistryResponse::known(true, removed)
        }
        // ── Edge registry ──────────────────────────────────────────────────────
        RegistryCommand::RegisterEdge { record } => {
            data.edges.insert(record.edge_id.clone(), record);
            RegistryResponse::known(true, 0)
        }
        RegistryCommand::EdgeHeartbeat { edge_id, active_calls, at_ms } => {
            if let Some(e) = data.edges.get_mut(&edge_id) {
                e.active_calls = active_calls;
                e.last_heartbeat_ms = at_ms;
                RegistryResponse::known(true, 0)
            } else {
                RegistryResponse::known(false, 0)
            }
        }
        RegistryCommand::RemoveEdge { edge_id } => {
            let removed = data.edges.remove(&edge_id).is_some() as u32;
            RegistryResponse::known(removed > 0, removed)
        }
        RegistryCommand::ReapStaleEdges { before_ms } => {
            let before = data.edges.len();
            data.edges.retain(|_, e| e.last_heartbeat_ms >= before_ms);
            let removed = (before - data.edges.len()) as u32;
            RegistryResponse::known(true, removed)
        }
        // ── Per-tenant call slots ───────────────────────────────────────────────
        RegistryCommand::AcquireCallSlot { call_id, tenant_id, max, at_ms } => {
            let slots = &mut data.call_slots;
            // Idempotent: re-acquiring an existing call_id is always granted.
            if slots.contains_key(&call_id) {
                let count = tenant_slots(slots, tenant_id);
                return RegistryResponse { known: true, removed: 0, granted: true, count };
            }
            let current = tenant_slots(slots, tenant_id);
            // Enforce the cap (evaluated here, in log order, so it's linearizable).
            if let Some(m) = max
                && m > 0
                && current >= m
            {
                return RegistryResponse { known: true, removed: 0, granted: false, count: current };
            }
            slots.insert(call_id, CallSlotRecord { tenant_id, at_ms });
            RegistryResponse { known: true, removed: 0, granted: true, count: current + 1 }
        }
        RegistryCommand::ReleaseCallSlot { call_id } => {
            let removed = data.call_slots.remove(&call_id).is_some() as u32;
            RegistryResponse::known(removed > 0, removed)
        }
        RegistryCommand::ReapCallSlots { before_ms } => {
            let before = data.call_slots.len();
            data.call_slots.retain(|_, s| s.at_ms >= before_ms);
            let removed = (before - data.call_slots.len()) as u32;
            RegistryResponse::known(true, removed)
        }
    }
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachineStore {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<NodeId>> {
        let (last_applied, last_membership, workers, edges, call_slots) = {
            let data = self.data.lock().await;
            (
                data.last_applied,
                data.last_membership.clone(),
                data.workers.clone(),
                data.edges.clone(),
                data.call_slots.clone(),
            )
        };

        let snapshot_id = {
            let mut idx = self.snapshot_idx.lock().await;
            *idx += 1;
            match last_applied {
                Some(log_id) => format!("{}-{}-{}", log_id.leader_id, log_id.index, *idx),
                None => format!("--{}", *idx),
            }
        };

        let stored = StoredSnapshot {
            last_applied,
            last_membership: last_membership.clone(),
            workers,
            edges,
            call_slots,
        };
        let bytes = serde_json::to_vec(&stored)
            .map_err(|e| StorageIOError::write_snapshot(None, &e))?;

        let meta = SnapshotMeta {
            last_log_id: last_applied,
            last_membership,
            snapshot_id,
        };
        let snapshot = Snapshot {
            meta: meta.clone(),
            snapshot: Box::new(Cursor::new(bytes.clone())),
        };
        *self.current_snapshot.lock().await = Some(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(bytes)),
        });
        Ok(snapshot)
    }
}

impl RaftStateMachine<TypeConfig> for StateMachineStore {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<NodeId>>, StoredMembership<NodeId, openraft::BasicNode>), StorageError<NodeId>>
    {
        let data = self.data.lock().await;
        Ok((data.last_applied, data.last_membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<RegistryResponse>, StorageError<NodeId>>
    where
        I: IntoIterator<Item = openraft::Entry<TypeConfig>> + Send,
        I::IntoIter: Send,
    {
        let mut data = self.data.lock().await;
        let mut responses = Vec::new();
        for entry in entries {
            data.last_applied = Some(entry.log_id);
            match entry.payload {
                EntryPayload::Blank => {
                    responses.push(RegistryResponse::known(false, 0));
                }
                EntryPayload::Normal(cmd) => {
                    let resp = apply_command(&mut data, cmd);
                    responses.push(resp);
                }
                EntryPayload::Membership(mem) => {
                    data.last_membership = StoredMembership::new(Some(entry.log_id), mem);
                    responses.push(RegistryResponse::known(false, 0));
                }
            }
        }
        Ok(responses)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<Cursor<Vec<u8>>>, StorageError<NodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, openraft::BasicNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<NodeId>> {
        let bytes = snapshot.into_inner();
        let stored: StoredSnapshot = serde_json::from_slice(&bytes)
            .map_err(|e| StorageIOError::read_snapshot(Some(meta.signature()), &e))?;

        let mut data = self.data.lock().await;
        data.last_applied = stored.last_applied;
        data.last_membership = stored.last_membership;
        data.workers = stored.workers;
        data.edges = stored.edges;
        data.call_slots = stored.call_slots;
        drop(data);

        *self.current_snapshot.lock().await = Some(Snapshot {
            meta: meta.clone(),
            snapshot: Box::new(Cursor::new(bytes)),
        });
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<TypeConfig>>, StorageError<NodeId>> {
        let cur = self.current_snapshot.lock().await;
        Ok(cur.as_ref().map(|s| Snapshot {
            meta: s.meta.clone(),
            snapshot: Box::new(Cursor::new(s.snapshot.get_ref().clone())),
        }))
    }
}
