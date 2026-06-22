//! Shared gRPC/protobuf definitions for the RustPBX distributed components.
//!
//! The `.proto` sources live in `proto/` and are compiled once here (build.rs).
//! `rustpbx-control`, `rustpbx-edge` and `rustpbx-worker` all depend on this
//! crate instead of each carrying their own copy of the schema.
//!
//! Module names match the proto `package`:
//! - `control` → `rustpbx.control` (ControlPlane service)
//! - `edge`    → `rustpbx.edge`    (EdgeWorker service)
//! - `raft`    → `rustpbx.raft`    (inter-node Raft transport)

pub mod control {
    tonic::include_proto!("rustpbx.control");
}

pub mod edge {
    tonic::include_proto!("rustpbx.edge");
}

pub mod raft {
    tonic::include_proto!("rustpbx.raft");
}
