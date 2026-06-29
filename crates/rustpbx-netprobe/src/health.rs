//! Minimal dependency-free HTTP health server for edge/worker nodes.
//!
//! Serves two unauthenticated endpoints for k8s/LB probes:
//! - `GET /healthz` → 200 (liveness: the process is up)
//! - `GET /readyz`  → 200 when `ready` is set (e.g. registered with the
//!   control plane), else 503
//!
//! It's a tiny HTTP/1.1 responder over `tokio` (no axum/hyper) — the node SIP
//! servers don't otherwise speak HTTP.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serve the health endpoints on `addr` until the process exits. Spawn it:
/// `tokio::spawn(health::serve(addr, ready))`.
pub async fn serve(addr: SocketAddr, ready: Arc<AtomicBool>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "health endpoint listening (/healthz, /readyz)");
    loop {
        let (mut stream, _peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "health accept failed");
                continue;
            }
        };
        let ready = Arc::clone(&ready);
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            // Request line: "GET /path HTTP/1.1"
            let path = std::str::from_utf8(&buf[..n])
                .ok()
                .and_then(|s| s.split_whitespace().nth(1))
                .unwrap_or("/");
            let (status, body) = match path {
                "/healthz" => ("200 OK", "ok"),
                "/readyz" => {
                    if ready.load(Ordering::Relaxed) {
                        ("200 OK", "ready")
                    } else {
                        ("503 Service Unavailable", "not ready")
                    }
                }
                _ => ("404 Not Found", "not found"),
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes()).await;
        });
    }
}
