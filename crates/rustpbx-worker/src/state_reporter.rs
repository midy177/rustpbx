//! `CallStateReporter` — reports terminal call state to the Edge via the
//! EdgeWorker `CallStateUpdate` gRPC.
//!
//! This runs as a CDR hook (the clean cross-crate extension point), so it fires
//! when a call ends, sending `COMPLETED` (normal) or `FAILED` (error status) to
//! the configured Edge. Mid-call transitions (ringing/answered) flow over SIP;
//! this is the optional out-of-band terminal signal. Reporting is best-effort —
//! a failure to reach the Edge is logged, never blocks the call.

use anyhow::Result;
use async_trait::async_trait;
use rustpbx::callrecord::{CallRecord, CallRecordHook};
use rustpbx_proto::edge::{
    edge_worker_client::EdgeWorkerClient, CallState, CallStateEvent,
};
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
}

#[async_trait]
impl CallRecordHook for CallStateReporter {
    async fn on_record_completed(&self, record: &mut CallRecord) -> Result<()> {
        // Map terminal status to a CallState. 2xx → completed, else failed.
        let state = if (200..300).contains(&record.status_code) {
            CallState::Completed
        } else {
            CallState::Failed
        };

        let event = CallStateEvent {
            call_id: record.call_id.clone(),
            worker_id: self.worker_id.clone(),
            state: state as i32,
            hangup_cause: Some(record.status_code as u32),
            reason: record.hangup_reason.as_ref().map(|r| format!("{r:?}")),
        };

        // Best-effort: connect + send, log on failure but don't propagate.
        match EdgeWorkerClient::connect(self.edge_endpoint.clone()).await {
            Ok(mut client) => {
                if let Err(e) = client.call_state_update(event).await {
                    warn!(call_id = %record.call_id, error = %e, "CallStateUpdate to edge failed");
                } else {
                    debug!(call_id = %record.call_id, ?state, "reported call state to edge");
                }
            }
            Err(e) => {
                warn!(edge = %self.edge_endpoint, error = %e, "connect to edge for CallStateUpdate failed");
            }
        }
        Ok(())
    }
}
