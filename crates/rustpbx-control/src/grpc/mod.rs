pub mod control_plane;

// Include tonic-build generated code.
// The generated modules are named after the proto package: rustpbx.control
pub mod proto {
    pub mod control {
        tonic::include_proto!("rustpbx.control");
    }
    pub mod edge {
        tonic::include_proto!("rustpbx.edge");
    }
}
