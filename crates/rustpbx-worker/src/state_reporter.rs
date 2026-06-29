//! `CallStateReporter` — reports call lifecycle state to the Edge via the
//! EdgeWorker `CallStateUpdate` gRPC.
//!
//! This runs as a CDR hook (the clean cross-crate extension point), so it fires
//! when a call ends. The Worker reconstructs the lifecycle from CDR timestamps:
//! `RINGING` at call start, `ANSWERED` when present, then `COMPLETED` or
//! `FAILED`. Reporting is best-effort — a failure to reach the Edge is logged,
//! never blocks the call.

use anyhow::Result;
use async_trait::async_trait;
use rustpbx::callrecord::{CallRecord, CallRecordHook};
use rustpbx_proto::edge::{CallState, CallStateEvent, edge_worker_client::EdgeWorkerClient};
use tracing::{debug, warn};

pub struct CallStateReporter {
    /// Edge EdgeWorker gRPC endpoint (with scheme), e.g. `http://10.0.0.3:9092`.
    edge_endpoint: String,
    worker_id: String,
}

impl CallStateReporter {
    /// Build a reporter targeting `edge_addr` (host:port or full URL).
    pub fn new(edge_addr: &str, worker_id: String) -> Self {
        let edge_endpoint = if edge_addr.starts_with("http") {
            edge_addr.to_string()
        } else {
            format!("http://{edge_addr}")
        };
        Self {
            edge_endpoint,
            worker_id,
        }
    }

    fn event(
        &self,
        record: &CallRecord,
        state: CallState,
        event_time_unix_ms: i64,
    ) -> CallStateEvent {
        let terminal = matches!(state, CallState::Completed | CallState::Failed);
        CallStateEvent {
            call_id: record.call_id.clone(),
            worker_id: self.worker_id.clone(),
            state: state as i32,
            hangup_cause: terminal.then_some(record.status_code as u32),
            reason: terminal
                .then(|| record.hangup_reason.as_ref().map(|r| format!("{r:?}")))
                .flatten(),
            event_time_unix_ms: Some(event_time_unix_ms),
        }
    }

    fn events_for_record(&self, record: &CallRecord) -> Vec<CallStateEvent> {
        let terminal_state = if (200..300).contains(&record.status_code) {
            CallState::Completed
        } else {
            CallState::Failed
        };

        let mut events = vec![self.event(
            record,
            CallState::Ringing,
            record.start_time.timestamp_millis(),
        )];
        if let Some(answer_time) = record.answer_time {
            events.push(self.event(record, CallState::Answered, answer_time.timestamp_millis()));
        }
        events.push(self.event(record, terminal_state, record.end_time.timestamp_millis()));
        events
    }
}

#[async_trait]
impl CallRecordHook for CallStateReporter {
    async fn on_record_completed(&self, record: &mut CallRecord) -> Result<()> {
        let events = self.events_for_record(record);

        // Best-effort: connect + send, log on failure but don't propagate.
        match EdgeWorkerClient::connect(self.edge_endpoint.clone()).await {
            Ok(mut client) => {
                for event in events {
                    let state = CallState::try_from(event.state).unwrap_or(CallState::Failed);
                    if let Err(e) = client.call_state_update(event).await {
                        warn!(call_id = %record.call_id, ?state, error = %e, "CallStateUpdate to edge failed");
                    } else {
                        debug!(call_id = %record.call_id, ?state, "reported call state to edge");
                    }
                }
            }
            Err(e) => {
                warn!(edge = %self.edge_endpoint, error = %e, "connect to edge for CallStateUpdate failed");
            }
        }
        Ok(())
    }
}
