//! Raft state machine: the replicated worker registry.
//!
//! Applies [`RegistryCommand`]s in log order to an in-memory map of
//! [`WorkerRecord`]s. Snapshots are a serialized copy of the whole map plus the
//! last-applied log id and membership — small and cheap, since the registry is
//! at most a few hundred workers.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use openraft::EntryPayload;
use openraft::LogId;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::StoredMembership;
use openraft::storage::RaftSnapshotBuilder;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::types::{
    CallSlotRecord, CallStartRecord, EdgeRecord, ExtensionContactConflict,
    ExtensionContactConflictCandidate, ExtensionContactRecord, NodeId, RegistryCommand,
    RegistryResponse, TypeConfig, WorkerRecord,
};

const CPS_WINDOW_MS: i64 = 1_000;

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
    /// Recently accepted call starts keyed by call_id (for CPS enforcement).
    #[serde(default)]
    pub call_starts: BTreeMap<String, CallStartRecord>,
    /// Sticky routing map: affinity_key -> worker_id.
    #[serde(default)]
    pub worker_affinity: BTreeMap<String, String>,
    /// Optional sticky routing expiry: affinity_key -> unix millis.
    #[serde(default)]
    pub worker_affinity_expires: BTreeMap<String, i64>,
    /// Multi-member sticky routing map: affinity_key -> worker_id -> expires_at_ms.
    /// An expiry of `0` means the member does not expire by TTL.
    #[serde(default)]
    pub worker_affinity_members: BTreeMap<String, BTreeMap<String, i64>>,
    /// Contact metadata for extension affinity: affinity_key -> worker_id -> contact -> record.
    #[serde(default)]
    pub worker_affinity_contacts:
        BTreeMap<String, BTreeMap<String, BTreeMap<String, ExtensionContactRecord>>>,
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
    /// call_id -> recent accepted start (trunk CPS control)
    call_starts: BTreeMap<String, CallStartRecord>,
    /// affinity_key -> worker_id
    worker_affinity: BTreeMap<String, String>,
    /// affinity_key -> unix millis
    worker_affinity_expires: BTreeMap<String, i64>,
    /// affinity_key -> worker_id -> unix millis; 0 means no expiry
    worker_affinity_members: BTreeMap<String, BTreeMap<String, i64>>,
    /// affinity_key -> worker_id -> contact -> record
    worker_affinity_contacts:
        BTreeMap<String, BTreeMap<String, BTreeMap<String, ExtensionContactRecord>>>,
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

    /// Resolve a sticky worker binding by affinity key.
    pub async fn worker_for_affinity(&self, affinity_key: &str) -> Option<String> {
        self.worker_ids_for_affinity(affinity_key)
            .await
            .into_iter()
            .next()
    }

    /// Resolve sticky worker bindings by affinity key, excluding expired members.
    pub async fn worker_ids_for_affinity(&self, affinity_key: &str) -> Vec<String> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let data = self.data.lock().await;
        if let Some(members) = data.worker_affinity_members.get(affinity_key) {
            let contacts = data.worker_affinity_contacts.get(affinity_key);
            let mut ids: Vec<String> = members
                .iter()
                .filter(|&(_, expires_at_ms)| *expires_at_ms == 0 || *expires_at_ms > now_ms)
                .map(|(worker_id, _)| worker_id.clone())
                .collect();
            ids.sort_by_key(|worker_id| {
                (
                    std::cmp::Reverse(best_contact_q_milli(contacts, worker_id, now_ms)),
                    worker_id.clone(),
                )
            });
            return ids;
        }
        if data
            .worker_affinity_expires
            .get(affinity_key)
            .is_some_and(|expires_at_ms| *expires_at_ms <= now_ms)
        {
            return Vec::new();
        }
        data.worker_affinity
            .get(affinity_key)
            .cloned()
            .into_iter()
            .collect()
    }

    /// Resolve extension Contact metadata by worker for an affinity key.
    pub async fn contacts_for_affinity(
        &self,
        affinity_key: &str,
    ) -> BTreeMap<String, Vec<ExtensionContactRecord>> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let data = self.data.lock().await;
        let mut best_by_contact: BTreeMap<String, (String, ExtensionContactRecord)> =
            BTreeMap::new();
        let Some(members) = data.worker_affinity_contacts.get(affinity_key) else {
            return BTreeMap::new();
        };
        for (worker_id, contacts) in members {
            for record in contacts
                .values()
                .filter(|record| record.expires_at_ms == 0 || record.expires_at_ms > now_ms)
            {
                let candidate = (worker_id.clone(), record.clone());
                let dedup_key = contact_dedup_key(&record.contact);
                let replace = best_by_contact
                    .get(&dedup_key)
                    .is_none_or(|current| better_contact_owner(&candidate, current));
                if replace {
                    best_by_contact.insert(dedup_key, candidate);
                }
            }
        }

        let mut out: BTreeMap<String, Vec<ExtensionContactRecord>> = BTreeMap::new();
        for (worker_id, record) in best_by_contact.into_values() {
            out.entry(worker_id).or_default().push(record);
        }
        for records in out.values_mut() {
            records
                .sort_by_key(|record| (std::cmp::Reverse(record.q_milli), record.contact.clone()));
        }
        out
    }

    /// Current Contact ownership conflicts across all extension affinity keys.
    pub async fn extension_contact_conflicts(&self) -> Vec<ExtensionContactConflict> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let data = self.data.lock().await;
        let mut conflicts = Vec::new();

        for (affinity_key, worker_contacts) in &data.worker_affinity_contacts {
            let mut by_contact: BTreeMap<String, Vec<ExtensionContactConflictCandidate>> =
                BTreeMap::new();
            for (worker_id, contacts) in worker_contacts {
                for record in contacts
                    .values()
                    .filter(|record| record.expires_at_ms == 0 || record.expires_at_ms > now_ms)
                {
                    by_contact
                        .entry(contact_dedup_key(&record.contact))
                        .or_default()
                        .push(ExtensionContactConflictCandidate {
                            worker_id: worker_id.clone(),
                            contact: record.contact.clone(),
                            q_milli: record.q_milli,
                            expires_at_ms: record.expires_at_ms,
                        });
                }
            }

            for (contact_key, mut candidates) in by_contact {
                if candidates.len() < 2 {
                    continue;
                }
                candidates.sort_by_key(|candidate| {
                    (
                        std::cmp::Reverse(candidate.q_milli),
                        std::cmp::Reverse(candidate.expires_at_ms),
                        candidate.worker_id.clone(),
                    )
                });
                let selected = &candidates[0];
                conflicts.push(ExtensionContactConflict {
                    affinity_key: affinity_key.clone(),
                    contact_key,
                    selected_worker_id: selected.worker_id.clone(),
                    selected_contact: selected.contact.clone(),
                    candidates,
                });
            }
        }

        conflicts
            .sort_by_key(|conflict| (conflict.affinity_key.clone(), conflict.contact_key.clone()));
        conflicts
    }
}

fn better_contact_owner(
    candidate: &(String, ExtensionContactRecord),
    current: &(String, ExtensionContactRecord),
) -> bool {
    let (candidate_worker, candidate_record) = candidate;
    let (current_worker, current_record) = current;
    (
        candidate_record.q_milli,
        candidate_record.expires_at_ms,
        std::cmp::Reverse(candidate_worker),
    ) > (
        current_record.q_milli,
        current_record.expires_at_ms,
        std::cmp::Reverse(current_worker),
    )
}

fn contact_dedup_key(contact: &str) -> String {
    let mut parts = contact.split(';');
    let Some(uri) = parts.next() else {
        return String::new();
    };
    let mut key = uri.trim().to_ascii_lowercase();
    for part in parts {
        let Some(name) = part.split_once('=').map(|(name, _)| name.trim()) else {
            key.push(';');
            key.push_str(part.trim());
            continue;
        };
        if name.eq_ignore_ascii_case("q") || name.eq_ignore_ascii_case("expires") {
            continue;
        }
        key.push(';');
        key.push_str(part.trim());
    }
    key
}

fn remove_worker_from_affinity(data: &mut StateMachineData, worker_id: &str) {
    data.worker_affinity.retain(|_, id| id != worker_id);
    data.worker_affinity_members.retain(|key, members| {
        members.remove(worker_id);
        if let Some(contact_members) = data.worker_affinity_contacts.get_mut(key) {
            contact_members.remove(worker_id);
            if contact_members.is_empty() {
                data.worker_affinity_contacts.remove(key);
            }
        }
        if let Some(primary) = data.worker_affinity.get(key)
            && primary == worker_id
        {
            if let Some(next) = members.keys().next() {
                data.worker_affinity.insert(key.clone(), next.clone());
            } else {
                data.worker_affinity.remove(key);
            }
        }
        !members.is_empty()
    });
    data.worker_affinity_expires
        .retain(|key, _| data.worker_affinity.contains_key(key));
}

fn best_contact_q_milli(
    contacts: Option<&BTreeMap<String, BTreeMap<String, ExtensionContactRecord>>>,
    worker_id: &str,
    now_ms: i64,
) -> u16 {
    contacts
        .and_then(|members| members.get(worker_id))
        .into_iter()
        .flat_map(|contact_map| contact_map.values())
        .filter(|record| record.expires_at_ms == 0 || record.expires_at_ms > now_ms)
        .map(|record| record.q_milli)
        .max()
        .unwrap_or(1000)
}

/// Count a tenant's currently-held slots in the map.
fn tenant_slots(slots: &BTreeMap<String, CallSlotRecord>, tenant_id: i64) -> u32 {
    slots.values().filter(|s| s.tenant_id == tenant_id).count() as u32
}

/// Count a tenant+trunk's currently-held slots in the map.
fn trunk_slots(slots: &BTreeMap<String, CallSlotRecord>, tenant_id: i64, trunk_name: &str) -> u32 {
    slots
        .values()
        .filter(|s| s.tenant_id == tenant_id && s.trunk_name.as_deref() == Some(trunk_name))
        .count() as u32
}

/// Count a tenant+trunk's accepted starts in the current CPS window.
fn trunk_cps(
    starts: &BTreeMap<String, CallStartRecord>,
    tenant_id: i64,
    trunk_name: &str,
    since_ms: i64,
) -> u32 {
    starts
        .values()
        .filter(|s| {
            s.at_ms >= since_ms
                && s.tenant_id == tenant_id
                && s.trunk_name.as_deref() == Some(trunk_name)
        })
        .count() as u32
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
        RegistryCommand::Heartbeat {
            worker_id,
            active_calls,
            cpu_usage,
            at_ms,
        } => {
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
            if removed > 0 {
                remove_worker_from_affinity(data, &worker_id);
            }
            RegistryResponse::known(removed > 0, removed)
        }
        RegistryCommand::ReapStale { before_ms } => {
            let before = workers.len();
            workers.retain(|_, w| w.last_heartbeat_ms >= before_ms);
            let removed = (before - workers.len()) as u32;
            if removed > 0 {
                data.worker_affinity
                    .retain(|_, worker_id| workers.contains_key(worker_id));
                data.worker_affinity_members.retain(|key, members| {
                    members.retain(|worker_id, _| workers.contains_key(worker_id));
                    if let Some(primary) = data.worker_affinity.get(key)
                        && !members.contains_key(primary)
                    {
                        if let Some(next) = members.keys().next() {
                            data.worker_affinity.insert(key.clone(), next.clone());
                        } else {
                            data.worker_affinity.remove(key);
                        }
                    }
                    !members.is_empty()
                });
                data.worker_affinity_expires
                    .retain(|key, _| data.worker_affinity.contains_key(key));
            }
            RegistryResponse::known(true, removed)
        }
        // ── Edge registry ──────────────────────────────────────────────────────
        RegistryCommand::RegisterEdge { record } => {
            data.edges.insert(record.edge_id.clone(), record);
            RegistryResponse::known(true, 0)
        }
        RegistryCommand::EdgeHeartbeat {
            edge_id,
            active_calls,
            at_ms,
        } => {
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
        RegistryCommand::AcquireCallSlot {
            call_id,
            tenant_id,
            max,
            trunk_name,
            trunk_max,
            trunk_max_cps,
            at_ms,
        } => {
            data.call_starts
                .retain(|_, start| start.at_ms >= at_ms.saturating_sub(CPS_WINDOW_MS));
            let slots = &mut data.call_slots;
            let starts = &mut data.call_starts;
            // Idempotent: re-acquiring an existing call_id is always granted.
            if slots.contains_key(&call_id) {
                let count = tenant_slots(slots, tenant_id);
                let trunk_count = trunk_name
                    .as_deref()
                    .map(|name| trunk_slots(slots, tenant_id, name))
                    .unwrap_or(0);
                let trunk_cps_count = trunk_name
                    .as_deref()
                    .map(|name| {
                        trunk_cps(starts, tenant_id, name, at_ms.saturating_sub(CPS_WINDOW_MS))
                    })
                    .unwrap_or(0);
                return RegistryResponse {
                    known: true,
                    removed: 0,
                    granted: true,
                    count,
                    trunk_count,
                    trunk_cps_count,
                };
            }
            let current = tenant_slots(slots, tenant_id);
            // Enforce the cap (evaluated here, in log order, so it's linearizable).
            if let Some(m) = max
                && m > 0
                && current >= m
            {
                return RegistryResponse {
                    known: true,
                    removed: 0,
                    granted: false,
                    count: current,
                    trunk_count: 0,
                    trunk_cps_count: 0,
                };
            }
            let trunk_current = trunk_name
                .as_deref()
                .map(|name| trunk_slots(slots, tenant_id, name))
                .unwrap_or(0);
            let trunk_cps_current = trunk_name
                .as_deref()
                .map(|name| trunk_cps(starts, tenant_id, name, at_ms.saturating_sub(CPS_WINDOW_MS)))
                .unwrap_or(0);
            if let Some(m) = trunk_max
                && m > 0
                && trunk_current >= m
            {
                return RegistryResponse {
                    known: true,
                    removed: 0,
                    granted: false,
                    count: current,
                    trunk_count: trunk_current,
                    trunk_cps_count: trunk_cps_current,
                };
            }
            if let Some(m) = trunk_max_cps
                && m > 0
                && trunk_cps_current >= m
            {
                return RegistryResponse {
                    known: true,
                    removed: 0,
                    granted: false,
                    count: current,
                    trunk_count: trunk_current,
                    trunk_cps_count: trunk_cps_current,
                };
            }
            slots.insert(
                call_id.clone(),
                CallSlotRecord {
                    tenant_id,
                    trunk_name: trunk_name.clone(),
                    at_ms,
                },
            );
            starts.insert(
                call_id,
                CallStartRecord {
                    tenant_id,
                    trunk_name,
                    at_ms,
                },
            );
            RegistryResponse {
                known: true,
                removed: 0,
                granted: true,
                count: current + 1,
                trunk_count: trunk_current + 1,
                trunk_cps_count: trunk_cps_current + 1,
            }
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
        RegistryCommand::BindAffinity {
            affinity_key,
            worker_id,
            expires_at_ms,
        } => {
            let member_expires_at_ms = expires_at_ms.unwrap_or(0);
            data.worker_affinity
                .insert(affinity_key.clone(), worker_id.clone());
            if expires_at_ms.is_none() {
                data.worker_affinity_expires.remove(&affinity_key);
            }
            data.worker_affinity_members
                .entry(affinity_key)
                .or_default()
                .insert(worker_id, member_expires_at_ms);
            RegistryResponse::known(true, 0)
        }
        RegistryCommand::BindAffinityContact {
            affinity_key,
            worker_id,
            contact,
            q_milli,
            expires_at_ms,
        } => {
            data.worker_affinity
                .insert(affinity_key.clone(), worker_id.clone());
            data.worker_affinity_members
                .entry(affinity_key.clone())
                .or_default()
                .insert(worker_id.clone(), expires_at_ms);
            data.worker_affinity_contacts
                .entry(affinity_key)
                .or_default()
                .entry(worker_id)
                .or_default()
                .insert(
                    contact.clone(),
                    ExtensionContactRecord {
                        contact,
                        q_milli: q_milli.min(1000),
                        expires_at_ms,
                    },
                );
            RegistryResponse::known(true, 0)
        }
        RegistryCommand::UnbindAffinity { affinity_key } => {
            let removed = data.worker_affinity.remove(&affinity_key).is_some() as u32;
            data.worker_affinity_expires.remove(&affinity_key);
            data.worker_affinity_members.remove(&affinity_key);
            data.worker_affinity_contacts.remove(&affinity_key);
            RegistryResponse::known(removed > 0, removed)
        }
        RegistryCommand::UnbindAffinityWorker {
            affinity_key,
            worker_id,
        } => {
            let mut removed = 0;
            if let Some(members) = data.worker_affinity_members.get_mut(&affinity_key) {
                removed = members.remove(&worker_id).is_some() as u32;
                if let Some(contact_members) = data.worker_affinity_contacts.get_mut(&affinity_key)
                {
                    contact_members.remove(&worker_id);
                    if contact_members.is_empty() {
                        data.worker_affinity_contacts.remove(&affinity_key);
                    }
                }
                if members.is_empty() {
                    data.worker_affinity_members.remove(&affinity_key);
                    data.worker_affinity.remove(&affinity_key);
                    data.worker_affinity_expires.remove(&affinity_key);
                    data.worker_affinity_contacts.remove(&affinity_key);
                } else if data.worker_affinity.get(&affinity_key) == Some(&worker_id)
                    && let Some(next) = members.keys().next()
                {
                    data.worker_affinity
                        .insert(affinity_key.clone(), next.clone());
                }
            } else if data.worker_affinity.get(&affinity_key) == Some(&worker_id) {
                removed = data.worker_affinity.remove(&affinity_key).is_some() as u32;
                data.worker_affinity_expires.remove(&affinity_key);
            }
            RegistryResponse::known(removed > 0, removed)
        }
        RegistryCommand::ReapAffinity { before_ms } => {
            let expired: Vec<String> = data
                .worker_affinity_expires
                .iter()
                .filter(|(key, _)| !data.worker_affinity_members.contains_key(*key))
                .filter(|&(_, expires_at_ms)| *expires_at_ms <= before_ms)
                .map(|(key, _)| key.clone())
                .collect();
            for key in &expired {
                data.worker_affinity.remove(key);
                data.worker_affinity_expires.remove(key);
                data.worker_affinity_members.remove(key);
                data.worker_affinity_contacts.remove(key);
            }
            let mut removed = expired.len() as u32;
            let keys: Vec<String> = data.worker_affinity_members.keys().cloned().collect();
            for key in keys {
                let Some(members) = data.worker_affinity_members.get_mut(&key) else {
                    continue;
                };
                let before = members.len();
                members
                    .retain(|_, expires_at_ms| *expires_at_ms == 0 || *expires_at_ms > before_ms);
                removed += (before - members.len()) as u32;
                if members.is_empty() {
                    data.worker_affinity_members.remove(&key);
                    data.worker_affinity.remove(&key);
                    data.worker_affinity_expires.remove(&key);
                    data.worker_affinity_contacts.remove(&key);
                } else if data
                    .worker_affinity
                    .get(&key)
                    .is_none_or(|primary| !members.contains_key(primary))
                    && let Some(next) = members.keys().next()
                {
                    data.worker_affinity.insert(key, next.clone());
                }
            }
            let contact_keys: Vec<String> = data.worker_affinity_contacts.keys().cloned().collect();
            for key in contact_keys {
                let Some(worker_contacts) = data.worker_affinity_contacts.get_mut(&key) else {
                    continue;
                };
                let workers: Vec<String> = worker_contacts.keys().cloned().collect();
                for worker_id in workers {
                    let Some(contacts) = worker_contacts.get_mut(&worker_id) else {
                        continue;
                    };
                    let before = contacts.len();
                    contacts.retain(|_, record| {
                        record.expires_at_ms == 0 || record.expires_at_ms > before_ms
                    });
                    removed += (before - contacts.len()) as u32;
                    if contacts.is_empty() {
                        worker_contacts.remove(&worker_id);
                    }
                }
                if worker_contacts.is_empty() {
                    data.worker_affinity_contacts.remove(&key);
                }
            }
            RegistryResponse::known(true, removed)
        }
    }
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachineStore {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<NodeId>> {
        let (
            last_applied,
            last_membership,
            workers,
            edges,
            call_slots,
            call_starts,
            worker_affinity,
            worker_affinity_expires,
            worker_affinity_members,
            worker_affinity_contacts,
        ) = {
            let data = self.data.lock().await;
            (
                data.last_applied,
                data.last_membership.clone(),
                data.workers.clone(),
                data.edges.clone(),
                data.call_slots.clone(),
                data.call_starts.clone(),
                data.worker_affinity.clone(),
                data.worker_affinity_expires.clone(),
                data.worker_affinity_members.clone(),
                data.worker_affinity_contacts.clone(),
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
            call_starts,
            worker_affinity,
            worker_affinity_expires,
            worker_affinity_members,
            worker_affinity_contacts,
        };
        let bytes =
            serde_json::to_vec(&stored).map_err(|e| StorageIOError::write_snapshot(None, &e))?;

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
    ) -> Result<
        (
            Option<LogId<NodeId>>,
            StoredMembership<NodeId, openraft::BasicNode>,
        ),
        StorageError<NodeId>,
    > {
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
        data.call_starts = stored.call_starts;
        data.worker_affinity = stored.worker_affinity;
        data.worker_affinity_expires = stored.worker_affinity_expires;
        data.worker_affinity_members = stored.worker_affinity_members;
        data.worker_affinity_contacts = stored.worker_affinity_contacts;
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
