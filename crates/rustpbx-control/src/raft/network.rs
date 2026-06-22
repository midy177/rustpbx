//! Raft network layer.
//!
//! Phase 1 runs a single-voter Raft, so no peer RPCs are ever sent — this node
//! is the only member and reaches quorum by itself. The trait must still be
//! implemented to construct `Raft`, so every method returns an "unreachable"
//! network error. When multi-replica membership is added (a later phase), this
//! is where outbound calls get wired over the existing tonic gRPC transport.

use openraft::error::InstallSnapshotError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::Unreachable;
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

use super::types::{NodeId, TypeConfig};

/// Factory that produces per-target network clients. Single-node: never used to
/// actually reach a peer, but required by `Raft::new`.
#[derive(Clone, Debug, Default)]
pub struct NetworkFactory;

impl RaftNetworkFactory<TypeConfig> for NetworkFactory {
    type Network = NetworkConnection;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Self::Network {
        NetworkConnection {
            target,
            addr: node.addr.clone(),
        }
    }
}

/// A connection to a single peer. In single-node mode no method is ever invoked
/// (there are no peers); each returns `Unreachable` defensively.
#[derive(Clone, Debug)]
pub struct NetworkConnection {
    #[allow(dead_code)]
    target: NodeId,
    #[allow(dead_code)]
    addr: String,
}

fn unreachable<E>() -> RPCError<NodeId, BasicNode, E>
where
    E: std::error::Error,
{
    RPCError::Unreachable(Unreachable::new(&std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "single-node raft has no peers; network RPC not available",
    )))
}

impl RaftNetwork<TypeConfig> for NetworkConnection {
    async fn append_entries(
        &mut self,
        _rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        Err(unreachable())
    }

    async fn install_snapshot(
        &mut self,
        _rpc: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<NodeId>,
        RPCError<NodeId, BasicNode, RaftError<NodeId, InstallSnapshotError>>,
    > {
        Err(unreachable())
    }

    async fn vote(
        &mut self,
        _rpc: VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        Err(unreachable())
    }
}
