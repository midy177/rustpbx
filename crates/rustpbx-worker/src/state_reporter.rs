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

        let ring_time = record.ring_time.unwrap_or(record.start_time);
        let mut events = vec![self.event(record, CallState::Ringing, ring_time.timestamp_millis())];
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use rustpbx::callrecord::{CallRecord, CallRecordHangupReason};

    fn state(event: &CallStateEvent) -> CallState {
        CallState::try_from(event.state).unwrap()
    }

    fn record(status_code: u16, answered: bool) -> CallRecord {
        let start = Utc.timestamp_millis_opt(1_000).unwrap();
        let ring = Utc.timestamp_millis_opt(1_500).unwrap();
        let answer = Utc.timestamp_millis_opt(2_000).unwrap();
        CallRecord {
            call_id: "call-1".to_string(),
            start_time: start,
            ring_time: Some(ring),
            answer_time: answered.then_some(answer),
            end_time: start + Duration::seconds(5),
            caller: "sip:1001@example.test".to_string(),
            callee: "sip:1002@example.test".to_string(),
            status_code,
            hangup_reason: (status_code >= 300).then_some(CallRecordHangupReason::Failed),
            ..Default::default()
        }
    }

    #[test]
    fn events_for_completed_call_include_ringing_answered_completed() {
        let reporter = CallStateReporter::new("127.0.0.1:9093", "worker-a".to_string());
        let events = reporter.events_for_record(&record(200, true));

        assert_eq!(events.len(), 3);
        assert_eq!(state(&events[0]), CallState::Ringing);
        assert_eq!(events[0].event_time_unix_ms, Some(1_500));
        assert_eq!(events[0].hangup_cause, None);

        assert_eq!(state(&events[1]), CallState::Answered);
        assert_eq!(events[1].event_time_unix_ms, Some(2_000));
        assert_eq!(events[1].hangup_cause, None);

        assert_eq!(state(&events[2]), CallState::Completed);
        assert_eq!(events[2].event_time_unix_ms, Some(6_000));
        assert_eq!(events[2].hangup_cause, Some(200));
    }

    #[test]
    fn events_for_failed_unanswered_call_skip_answered() {
        let reporter = CallStateReporter::new("127.0.0.1:9093", "worker-a".to_string());
        let events = reporter.events_for_record(&record(486, false));

        assert_eq!(events.len(), 2);
        assert_eq!(state(&events[0]), CallState::Ringing);
        assert_eq!(state(&events[1]), CallState::Failed);
        assert_eq!(events[1].event_time_unix_ms, Some(6_000));
        assert_eq!(events[1].hangup_cause, Some(486));
        assert!(events[1].reason.as_deref().unwrap_or("").contains("Failed"));
    }
}
