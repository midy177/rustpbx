//! Edge-side EdgeWorker gRPC service.
//!
//! Implements `CallStateUpdate`: Workers report call-state transitions
//! (ringing/answered/completed/failed/transferred) out-of-band. This is an
//! optional control channel — the authoritative call teardown still happens
//! over SIP — so here we keep a small in-memory latest-state table for
//! observability and log each transition.
//!
//! `AllocateCall` is the *opposite* direction (Edge → Worker), so its server
//! lives on the Worker; here it's unimplemented.

use std::{collections::HashMap, sync::Arc};

use rustpbx_proto::edge::{
    AllocateCallRequest, AllocateCallResponse, CallState, CallStateAck, CallStateEvent,
    edge_worker_server::EdgeWorker,
};
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

const DEFAULT_MAX_CALL_STATES: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub struct EdgeCallState {
    pub call_id: String,
    pub worker_id: String,
    pub state: CallState,
    pub hangup_cause: Option<u32>,
    pub reason: Option<String>,
    pub event_time_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug)]
pub struct EdgeCallStateStore {
    inner: Arc<RwLock<HashMap<String, EdgeCallState>>>,
    max_entries: usize,
}

impl EdgeCallStateStore {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            max_entries: max_entries.max(1),
        }
    }

    pub async fn record(&self, event: &CallStateEvent) -> EdgeCallState {
        let state = CallState::try_from(event.state).unwrap_or(CallState::Failed);
        let event_time_unix_ms = event
            .event_time_unix_ms
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        let now_ms = chrono::Utc::now().timestamp_millis();
        let next = EdgeCallState {
            call_id: event.call_id.clone(),
            worker_id: event.worker_id.clone(),
            state,
            hangup_cause: event.hangup_cause,
            reason: event.reason.clone(),
            event_time_unix_ms,
            updated_at_unix_ms: now_ms,
        };

        let mut states = self.inner.write().await;
        let stored = match states.get(&event.call_id) {
            Some(current) if is_newer_or_equal_priority(&next, current) => {
                states.insert(event.call_id.clone(), next.clone());
                next
            }
            Some(current) => current.clone(),
            None => {
                states.insert(event.call_id.clone(), next.clone());
                next
            }
        };
        prune_oldest(&mut states, self.max_entries);
        stored
    }

    #[cfg(test)]
    pub async fn get(&self, call_id: &str) -> Option<EdgeCallState> {
        self.inner.read().await.get(call_id).cloned()
    }

    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

impl Default for EdgeCallStateStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CALL_STATES)
    }
}

fn is_newer_or_equal_priority(next: &EdgeCallState, current: &EdgeCallState) -> bool {
    next.event_time_unix_ms > current.event_time_unix_ms
        || (next.event_time_unix_ms == current.event_time_unix_ms
            && state_rank(next.state) >= state_rank(current.state))
}

fn state_rank(state: CallState) -> u8 {
    match state {
        CallState::Ringing => 0,
        CallState::Answered => 1,
        CallState::Transferred => 2,
        CallState::Completed | CallState::Failed => 3,
    }
}

fn prune_oldest(states: &mut HashMap<String, EdgeCallState>, max_entries: usize) {
    while states.len() > max_entries {
        let Some(oldest) = states
            .iter()
            .min_by_key(|(_, state)| (state.updated_at_unix_ms, state.call_id.clone()))
            .map(|(call_id, _)| call_id.clone())
        else {
            break;
        };
        states.remove(&oldest);
    }
}

/// Edge-side EdgeWorker service (CallStateUpdate receiver).
#[derive(Clone, Default)]
pub struct EdgeWorkerServer {
    state_store: EdgeCallStateStore,
}

impl EdgeWorkerServer {
    pub fn new(state_store: EdgeCallStateStore) -> Self {
        Self { state_store }
    }
}

#[tonic::async_trait]
impl EdgeWorker for EdgeWorkerServer {
    async fn allocate_call(
        &self,
        _request: Request<AllocateCallRequest>,
    ) -> Result<Response<AllocateCallResponse>, Status> {
        // Edge is the *client* for AllocateCall (it asks Workers). The server
        // side lives on the Worker.
        Err(Status::unimplemented(
            "AllocateCall server runs on the Worker, not the Edge",
        ))
    }

    async fn call_state_update(
        &self,
        request: Request<CallStateEvent>,
    ) -> Result<Response<CallStateAck>, Status> {
        let ev = request.into_inner();
        let state = CallState::try_from(ev.state).unwrap_or(CallState::Failed);
        let stored = self.state_store.record(&ev).await;
        info!(
            call_id = %ev.call_id,
            worker_id = %ev.worker_id,
            ?state,
            event_time_unix_ms = ev.event_time_unix_ms,
            hangup_cause = ev.hangup_cause,
            reason = ev.reason.as_deref().unwrap_or(""),
            latest_state = ?stored.state,
            "worker reported call state"
        );
        if matches!(state, CallState::Failed) {
            warn!(call_id = %ev.call_id, "worker reported call failure");
        }
        Ok(Response::new(CallStateAck { received: true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(call_id: &str, state: CallState, at_ms: i64) -> CallStateEvent {
        CallStateEvent {
            call_id: call_id.to_string(),
            worker_id: "worker-a".to_string(),
            state: state as i32,
            hangup_cause: None,
            reason: None,
            event_time_unix_ms: Some(at_ms),
        }
    }

    #[tokio::test]
    async fn call_state_update_records_latest_state() {
        let store = EdgeCallStateStore::new(16);
        let svc = EdgeWorkerServer::new(store.clone());

        svc.call_state_update(Request::new(event("call-1", CallState::Ringing, 100)))
            .await
            .unwrap();
        svc.call_state_update(Request::new(event("call-1", CallState::Answered, 200)))
            .await
            .unwrap();

        let state = store.get("call-1").await.unwrap();
        assert_eq!(state.state, CallState::Answered);
        assert_eq!(state.event_time_unix_ms, 200);
    }

    #[tokio::test]
    async fn call_state_update_ignores_older_events() {
        let store = EdgeCallStateStore::new(16);
        let svc = EdgeWorkerServer::new(store.clone());

        svc.call_state_update(Request::new(event("call-1", CallState::Completed, 300)))
            .await
            .unwrap();
        svc.call_state_update(Request::new(event("call-1", CallState::Ringing, 100)))
            .await
            .unwrap();

        let state = store.get("call-1").await.unwrap();
        assert_eq!(state.state, CallState::Completed);
        assert_eq!(state.event_time_unix_ms, 300);
    }

    #[tokio::test]
    async fn call_state_store_prunes_oldest_entries() {
        let store = EdgeCallStateStore::new(2);

        store
            .record(&event("call-1", CallState::Ringing, 100))
            .await;
        store
            .record(&event("call-2", CallState::Ringing, 200))
            .await;
        store
            .record(&event("call-3", CallState::Ringing, 300))
            .await;

        assert_eq!(store.len().await, 2);
        assert!(store.get("call-1").await.is_none());
        assert!(store.get("call-2").await.is_some());
        assert!(store.get("call-3").await.is_some());
    }
}
