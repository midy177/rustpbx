//! Raft network layer — inter-node transport over a dedicated gRPC service.
//!
//! Each openraft `RaftNetwork` method serializes its request to JSON, ships it
//! through the `RaftService` gRPC RPC (bytes in / bytes out), and deserializes
//! the reply. The reply is a [`WireResult`] carrying either the openraft
//! response or a remote `RaftError`, so a follower's logical errors propagate
//! back to the leader as `RemoteError` rather than being lost.
//!
//! Peer addresses come from the `BasicNode.addr` openraft hands us in
//! `new_client` (set when a node joins via `add_learner`). With no peers
//! configured this layer is simply never exercised (single-node mode).

use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::RemoteError;
use openraft::network::RPCOption;
use openraft::network::RaftNetwork;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::BasicNode;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::grpc::proto::raft::raft_service_client::RaftServiceClient;
use crate::grpc::proto::raft::RaftBytes;

use super::types::{NodeId, TypeConfig};

/// Wire envelope for a remote handler's result: either the typed response `T`
/// or the openraft `RaftError<E>` the remote produced. Both sides agree on JSON.
#[derive(Serialize, serde::Deserialize)]
pub enum WireResult<T, E> {
    Ok(T),
    Err(E),
}

/// Factory that dials peers on demand. Carries the optional client TLS config
/// used to verify peer Raft servers; the target address comes from the
/// `BasicNode` openraft passes to `new_client`.
#[derive(Clone, Debug, Default)]
pub struct NetworkFactory {
    tls: Option<tonic::transport::ClientTlsConfig>,
}

impl NetworkFactory {
    pub fn new(tls: Option<tonic::transport::ClientTlsConfig>) -> Self {
        Self { tls }
    }
}

impl RaftNetworkFactory<TypeConfig> for NetworkFactory {
    type Network = NetworkConnection;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Self::Network {
        NetworkConnection {
            target,
            // node.addr packs "raft_addr|grpc_addr"; dial the Raft transport.
            addr: super::types::node_addr::raft_addr(&node.addr).to_string(),
            tls: self.tls.clone(),
        }
    }
}

/// A lazily-connected link to one peer. A fresh tonic channel is opened per RPC
/// for simplicity — Raft RPC volume is modest and tonic pools the underlying
/// HTTP/2 connection by URI.
#[derive(Clone, Debug)]
pub struct NetworkConnection {
    target: NodeId,
    addr: String,
    tls: Option<tonic::transport::ClientTlsConfig>,
}

impl NetworkConnection {
    fn endpoint(&self) -> String {
        // Prefix the scheme tonic expects; TLS → https.
        if self.addr.starts_with("http://") || self.addr.starts_with("https://") {
            self.addr.clone()
        } else if self.tls.is_some() {
            format!("https://{}", self.addr)
        } else {
            format!("http://{}", self.addr)
        }
    }

    async fn connect(
        &self,
    ) -> Result<RaftServiceClient<tonic::transport::Channel>, NetworkError> {
        let mut endpoint = tonic::transport::Endpoint::from_shared(self.endpoint())
            .map_err(|e| NetworkError::new(&e))?;
        if let Some(tls) = &self.tls {
            endpoint = endpoint
                .tls_config(tls.clone())
                .map_err(|e| NetworkError::new(&e))?;
        }
        let channel = endpoint.connect().await.map_err(|e| NetworkError::new(&e))?;
        Ok(RaftServiceClient::new(channel))
    }
}

/// Serialize a request to JSON bytes, mapping failures to `NetworkError`.
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, NetworkError> {
    serde_json::to_vec(value).map_err(|e| NetworkError::new(&e))
}

/// Decode a `WireResult` reply into the openraft `Result<T, RPCError>` shape.
fn decode_reply<T, E>(
    target: NodeId,
    bytes: &[u8],
) -> Result<T, RPCError<NodeId, BasicNode, E>>
where
    T: DeserializeOwned,
    E: DeserializeOwned + std::error::Error,
{
    let wire: WireResult<T, E> =
        serde_json::from_slice(bytes).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
    match wire {
        WireResult::Ok(resp) => Ok(resp),
        WireResult::Err(remote) => Err(RPCError::RemoteError(RemoteError::new(target, remote))),
    }
}

impl RaftNetwork<TypeConfig> for NetworkConnection {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        let data = encode(&rpc).map_err(RPCError::Network)?;
        let mut client = self.connect().await.map_err(RPCError::Network)?;
        let reply = client
            .append_entries(RaftBytes { data })
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        decode_reply(self.target, &reply.into_inner().data)
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<NodeId>,
        RPCError<NodeId, BasicNode, RaftError<NodeId, InstallSnapshotError>>,
    > {
        let data = encode(&rpc).map_err(RPCError::Network)?;
        let mut client = self.connect().await.map_err(RPCError::Network)?;
        let reply = client
            .install_snapshot(RaftBytes { data })
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        decode_reply(self.target, &reply.into_inner().data)
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        let data = encode(&rpc).map_err(RPCError::Network)?;
        let mut client = self.connect().await.map_err(RPCError::Network)?;
        let reply = client
            .vote(RaftBytes { data })
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        decode_reply(self.target, &reply.into_inner().data)
    }
}
