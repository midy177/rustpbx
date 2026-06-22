//! Single-node Raft for replicating the worker registry across control replicas.
//!
//! Phase 1 runs a single-voter Raft (this node is the only member) so behaviour
//! is identical to the previous in-memory registry while establishing the Raft
//! plumbing. Multi-replica membership is wired in a later phase.

pub mod log_store;
pub mod network;
pub mod registry;
pub mod state_machine;
pub mod types;

// Re-exported for use by the storage/network/state-machine modules added in the
// following Phase 1 steps; allow unused until those land.
#[allow(unused_imports)]
pub use types::{NodeId, RegistryCommand, RegistryResponse, TypeConfig, WorkerRecord};
