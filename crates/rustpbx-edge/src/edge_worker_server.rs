//! Edge-side EdgeWorker gRPC service.
//!
//! Implements `CallStateUpdate`: Workers report call-state transitions
//! (ringing/answered/completed/failed/transferred) out-of-band. This is an
//! optional control channel — the authoritative call teardown still happens
//! over SIP — so here we observe and log; hooks for metrics/CDR correlation can
//! be added later.
//!
//! `AllocateCall` is the *opposite* direction (Edge → Worker), so its server
//! lives on the Worker; here it's unimplemented.

use rustpbx_proto::edge::{
    edge_worker_server::EdgeWorker, AllocateCallRequest, AllocateCallResponse, CallState,
    CallStateAck, CallStateEvent,
};
use tonic::{Request, Response, Status};
use tracing::{info, warn};

/// Edge-side EdgeWorker service (CallStateUpdate receiver).
#[derive(Default)]
pub struct EdgeWorkerServer;

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
        info!(
            call_id = %ev.call_id,
            worker_id = %ev.worker_id,
            ?state,
            hangup_cause = ev.hangup_cause,
            reason = ev.reason.as_deref().unwrap_or(""),
            "worker reported call state"
        );
        if matches!(state, CallState::Failed) {
            warn!(call_id = %ev.call_id, "worker reported call failure");
        }
        Ok(Response::new(CallStateAck { received: true }))
    }
}
