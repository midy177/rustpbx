//! Worker-side EdgeWorker gRPC service.
//!
//! Implements `AllocateCall`: when the Edge picks this Worker for an incoming
//! call, it calls here first. We reserve a slot (so concurrent allocations
//! don't oversubscribe between selection and INVITE arrival) and return our
//! internal SIP contact, where the Edge then sends the INVITE.
//!
//! `CallStateUpdate` is the *opposite* direction (Worker → Edge), so its server
//! lives on the Edge; here it's unimplemented.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use rustpbx_proto::edge::{
    edge_worker_server::EdgeWorker, AllocateCallRequest, AllocateCallResponse, CallStateAck,
    CallStateEvent,
};
use tonic::{Request, Response, Status};
use tracing::info;

use crate::reservations::CallReservations;

/// Worker-side EdgeWorker service.
pub struct EdgeWorkerService {
    reservations: CallReservations,
    /// Our internal SIP contact, e.g. `sip:10.0.0.7:5060`, returned to the Edge.
    sip_contact: String,
    /// Capacity guard: reject allocations once active >= max_concurrent.
    active_calls: Arc<AtomicU32>,
    max_concurrent: u32,
}

impl EdgeWorkerService {
    pub fn new(
        reservations: CallReservations,
        sip_contact: String,
        active_calls: Arc<AtomicU32>,
        max_concurrent: u32,
    ) -> Self {
        Self {
            reservations,
            sip_contact,
            active_calls,
            max_concurrent,
        }
    }
}

#[tonic::async_trait]
impl EdgeWorker for EdgeWorkerService {
    async fn allocate_call(
        &self,
        request: Request<AllocateCallRequest>,
    ) -> Result<Response<AllocateCallResponse>, Status> {
        let req = request.into_inner();

        // Capacity check (0 = unlimited). Reservations count toward active_calls,
        // so this naturally rejects once the worker is full.
        if self.max_concurrent > 0
            && self.active_calls.load(Ordering::Relaxed) >= self.max_concurrent
        {
            info!(call_id = %req.call_id, "allocate rejected — worker at capacity");
            return Ok(Response::new(AllocateCallResponse {
                accepted: false,
                worker_sip_contact: String::new(),
                reject_reason: Some("worker at capacity".to_string()),
            }));
        }

        self.reservations.reserve(&req.call_id);
        info!(
            call_id = %req.call_id,
            tenant_id = req.tenant_id,
            direction = %req.direction,
            contact = %self.sip_contact,
            "allocated call slot"
        );

        Ok(Response::new(AllocateCallResponse {
            accepted: true,
            worker_sip_contact: self.sip_contact.clone(),
            reject_reason: None,
        }))
    }

    async fn call_state_update(
        &self,
        _request: Request<CallStateEvent>,
    ) -> Result<Response<CallStateAck>, Status> {
        // Worker is the *client* for CallStateUpdate (it reports to the Edge).
        // The server side lives on the Edge.
        Err(Status::unimplemented(
            "CallStateUpdate server runs on the Edge, not the Worker",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustpbx_proto::edge::edge_worker_client::EdgeWorkerClient;
    use rustpbx_proto::edge::edge_worker_server::EdgeWorkerServer;

    fn req(call_id: &str) -> AllocateCallRequest {
        AllocateCallRequest {
            call_id: call_id.into(),
            tenant_id: 0,
            trunk_name: String::new(),
            caller: "sip:a@x".into(),
            callee: "sip:b@y".into(),
            direction: "inbound".into(),
            custom_headers: Default::default(),
        }
    }

    /// End-to-end over real gRPC: Edge calls AllocateCall, Worker reserves a
    /// slot (active_calls++), returns its SIP contact, and the reservation is
    /// then claimable on INVITE arrival (no double count).
    #[tokio::test]
    async fn allocate_call_reserves_and_returns_contact() {
        let active = Arc::new(AtomicU32::new(0));
        let reservations = CallReservations::new(Arc::clone(&active), 30_000);
        let svc = EdgeWorkerService::new(
            reservations.clone(),
            "sip:10.0.0.7:5060".into(),
            Arc::clone(&active),
            100,
        );

        let addr: std::net::SocketAddr = "127.0.0.1:24131".parse().unwrap();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(EdgeWorkerServer::new(svc))
                .serve(addr)
                .await
                .unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let mut client = EdgeWorkerClient::connect("http://127.0.0.1:24131")
            .await
            .unwrap();
        let resp = client.allocate_call(req("call-1")).await.unwrap().into_inner();

        assert!(resp.accepted);
        assert_eq!(resp.worker_sip_contact, "sip:10.0.0.7:5060");
        assert_eq!(active.load(Ordering::Relaxed), 1, "slot reserved");
        // INVITE arrival claims it — no second increment.
        assert!(reservations.claim("call-1"));
        assert_eq!(active.load(Ordering::Relaxed), 1);
    }

    /// At capacity, AllocateCall is rejected.
    #[tokio::test]
    async fn allocate_call_rejects_when_full() {
        let active = Arc::new(AtomicU32::new(2)); // already at max
        let reservations = CallReservations::new(Arc::clone(&active), 30_000);
        let svc = EdgeWorkerService::new(reservations, "sip:x:5060".into(), Arc::clone(&active), 2);

        let addr: std::net::SocketAddr = "127.0.0.1:24132".parse().unwrap();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(EdgeWorkerServer::new(svc))
                .serve(addr)
                .await
                .unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let mut client = EdgeWorkerClient::connect("http://127.0.0.1:24132")
            .await
            .unwrap();
        let resp = client.allocate_call(req("call-x")).await.unwrap().into_inner();
        assert!(!resp.accepted, "rejected at capacity");
        assert_eq!(active.load(Ordering::Relaxed), 2, "no slot reserved on reject");
    }
}
