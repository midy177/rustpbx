//! Raft gRPC server — the receiving end of inter-node Raft traffic.
//!
//! Implements the `RaftService` generated from `raft.proto`. Each RPC decodes
//! the JSON request, dispatches it to the local `Raft` handle, and encodes the
//! handler's `Result` back as a [`WireResult`] (so the caller can reconstruct a
//! `RemoteError` for logical failures). Runs on its own listener/port so Raft
//! traffic is isolated from the business gRPC service.

use openraft::raft::AppendEntriesRequest;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::VoteRequest;
use openraft::Raft;
use serde::Serialize;
use tonic::{Request, Response, Status};

use crate::grpc::proto::raft::raft_service_server::RaftService;
use crate::grpc::proto::raft::RaftBytes;

use super::network::WireResult;
use super::types::{NodeId, TypeConfig};

/// gRPC service wrapping the local Raft node.
pub struct RaftServer {
    raft: Raft<TypeConfig>,
}

impl RaftServer {
    pub fn new(raft: Raft<TypeConfig>) -> Self {
        Self { raft }
    }
}

/// Decode a JSON request body, returning a gRPC `invalid_argument` on failure.
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Status> {
    serde_json::from_slice(bytes).map_err(|e| Status::invalid_argument(format!("decode: {e}")))
}

/// Encode a handler `Result<T, E>` as a `WireResult` JSON body.
fn encode_result<T: Serialize, E: Serialize>(
    result: Result<T, E>,
) -> Result<Response<RaftBytes>, Status> {
    let wire = match result {
        Ok(v) => WireResult::<T, E>::Ok(v),
        Err(e) => WireResult::<T, E>::Err(e),
    };
    let data = serde_json::to_vec(&wire).map_err(|e| Status::internal(format!("encode: {e}")))?;
    Ok(Response::new(RaftBytes { data }))
}

#[tonic::async_trait]
impl RaftService for RaftServer {
    async fn append_entries(
        &self,
        request: Request<RaftBytes>,
    ) -> Result<Response<RaftBytes>, Status> {
        let req: AppendEntriesRequest<TypeConfig> = decode(&request.into_inner().data)?;
        let res = self.raft.append_entries(req).await;
        encode_result(res)
    }

    async fn vote(&self, request: Request<RaftBytes>) -> Result<Response<RaftBytes>, Status> {
        let req: VoteRequest<NodeId> = decode(&request.into_inner().data)?;
        let res = self.raft.vote(req).await;
        encode_result(res)
    }

    async fn install_snapshot(
        &self,
        request: Request<RaftBytes>,
    ) -> Result<Response<RaftBytes>, Status> {
        let req: InstallSnapshotRequest<TypeConfig> = decode(&request.into_inner().data)?;
        let res = self.raft.install_snapshot(req).await;
        encode_result(res)
    }
}
