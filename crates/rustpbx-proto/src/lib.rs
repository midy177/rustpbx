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

/// Shared TLS/transport helpers so the edge and worker build their Control
/// Plane channels the same way (custom-CA verification + optional client cert
/// for mutual TLS).
pub mod tls {
    use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

    /// Client-side TLS material, already loaded from disk by the caller.
    #[derive(Clone)]
    pub struct ClientTls {
        /// CA certificate (PEM) used to verify the Control Plane's server cert.
        pub ca_pem: Vec<u8>,
        /// Optional client cert + key (PEM) presented for mutual TLS.
        pub identity: Option<(Vec<u8>, Vec<u8>)>,
        /// Domain to verify against the server cert's SAN (overrides the host
        /// in the address — useful when connecting to an IP).
        pub domain: Option<String>,
    }

    impl ClientTls {
        /// Build the tonic `ClientTlsConfig` from this material.
        pub fn config(&self) -> ClientTlsConfig {
            let mut cfg = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(&self.ca_pem));
            if let Some((cert, key)) = &self.identity {
                cfg = cfg.identity(Identity::from_pem(cert, key));
            }
            if let Some(domain) = &self.domain {
                cfg = cfg.domain_name(domain);
            }
            cfg
        }
    }

    /// Boxed error so both `InvalidUri` (from `from_shared`) and
    /// `transport::Error` (from `tls_config`) fit one return type.
    pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

    /// Build a (not-yet-connected) `Endpoint` for `addr`, applying TLS when
    /// configured. A `None` `tls` yields a plaintext endpoint (backward
    /// compatible). Errors only on a malformed address or invalid TLS config.
    pub fn endpoint(addr: &str, tls: Option<&ClientTls>) -> Result<Endpoint, BoxError> {
        let ep = Channel::from_shared(addr.to_string())?;
        match tls {
            Some(t) => Ok(ep.tls_config(t.config())?),
            None => Ok(ep),
        }
    }
}
