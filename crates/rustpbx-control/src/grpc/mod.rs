pub mod control_plane;

// Re-export the shared generated protobuf code from `rustpbx-proto` so existing
// `crate::grpc::proto::{control,edge,raft}` paths keep resolving unchanged.
pub use rustpbx_proto as proto;
