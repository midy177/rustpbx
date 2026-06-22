//! In-memory Raft log store.
//!
//! Phase 1 keeps the Raft log and vote purely in memory: the worker registry is
//! ephemeral, high-churn state, and on a full cluster restart Workers simply
//! re-register. This mirrors the canonical openraft `memstore` example, adapted
//! to our [`TypeConfig`]. Persistence to disk is intentionally out of scope.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::RangeBounds;
use std::sync::Arc;

use openraft::storage::LogFlushed;
use openraft::storage::LogState;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftLogReader;
use openraft::LogId;
use openraft::StorageError;
use openraft::Vote;
use tokio::sync::Mutex;

use super::types::{NodeId, TypeConfig};

type Entry = openraft::Entry<TypeConfig>;

#[derive(Debug, Default)]
struct LogStoreInner {
    /// The Raft log, keyed by log index.
    log: BTreeMap<u64, Entry>,
    /// The last purged log id (logs up to and including this are deleted).
    last_purged_log_id: Option<LogId<NodeId>>,
    /// The latest persisted vote.
    vote: Option<Vote<NodeId>>,
    /// The last committed log id (optional persistence).
    committed: Option<LogId<NodeId>>,
}

/// In-memory log store. Cloneable handle around a shared, mutex-guarded inner.
#[derive(Clone, Debug, Default)]
pub struct LogStore {
    inner: Arc<Mutex<LogStoreInner>>,
}

impl LogStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RaftLogReader<TypeConfig> for LogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry>, StorageError<NodeId>> {
        let inner = self.inner.lock().await;
        let entries = inner
            .log
            .range(range)
            .map(|(_, entry)| entry.clone())
            .collect();
        Ok(entries)
    }
}

impl RaftLogStorage<TypeConfig> for LogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<NodeId>> {
        let inner = self.inner.lock().await;
        let last = inner.log.iter().next_back().map(|(_, e)| e.log_id);
        let last_purged = inner.last_purged_log_id;
        // last_log_id is the last present entry, or the last purged id if empty.
        let last_log_id = last.or(last_purged);
        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote<NodeId>) -> Result<(), StorageError<NodeId>> {
        self.inner.lock().await.vote = Some(*vote);
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<NodeId>>, StorageError<NodeId>> {
        Ok(self.inner.lock().await.vote)
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<NodeId>>,
    ) -> Result<(), StorageError<NodeId>> {
        self.inner.lock().await.committed = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<NodeId>>, StorageError<NodeId>> {
        Ok(self.inner.lock().await.committed)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry> + Send,
        I::IntoIter: Send,
    {
        {
            let mut inner = self.inner.lock().await;
            for entry in entries {
                inner.log.insert(entry.log_id.index, entry);
            }
        }
        // In-memory store: data is "persisted" the moment it's in the map.
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let mut inner = self.inner.lock().await;
        inner.log.split_off(&log_id.index);
        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let mut inner = self.inner.lock().await;
        inner.last_purged_log_id = Some(log_id);
        // Remove everything up to and including log_id.index.
        let keep = inner.log.split_off(&(log_id.index + 1));
        inner.log = keep;
        Ok(())
    }
}
