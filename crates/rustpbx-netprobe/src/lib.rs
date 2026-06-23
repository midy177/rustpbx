//! STUN-based public-IP and NAT-type detection.
//!
//! On startup an edge/worker calls [`probe`] with a couple of STUN servers. It
//! learns its **server-reflexive (public) address** and classifies its NAT's
//! *mapping behaviour* (RFC 5780 simplified) by comparing the reflexive address
//! seen from two different STUN servers using the **same local socket**:
//!
//! - same mapping for both servers  → endpoint-independent (`cone`)
//! - different mapping per server    → endpoint-dependent (`symmetric`, bad for
//!   peer-to-peer media)
//! - reflexive address == local IP   → no NAT (`open`)
//!
//! This is intentionally a lightweight, dependency-free STUN Binding client
//! (IPv4), not a full ICE agent.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;

const MAGIC_COOKIE: u32 = 0x2112_A442;

/// No NAT — the reflexive address equals the host's local address.
pub const NAT_OPEN: &str = "open";
/// Endpoint-independent mapping (full/restricted cone). Good for media P2P.
pub const NAT_CONE: &str = "cone";
/// Endpoint-dependent mapping. Breaks direct media; needs relay/anchoring.
pub const NAT_SYMMETRIC: &str = "symmetric";
/// Behind a NAT but only one STUN server answered, so cone vs symmetric is
/// undetermined.
pub const NAT_NAT: &str = "nat";
/// No STUN server answered (UDP blocked / unreachable).
pub const NAT_BLOCKED: &str = "blocked";
/// Probe not run or misconfigured.
pub const NAT_UNKNOWN: &str = "unknown";

/// Result of a NAT probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatInfo {
    /// Detected public (server-reflexive) IP, if any.
    pub public_ip: Option<String>,
    /// Detected public port, if any.
    pub public_port: Option<u16>,
    /// One of the `NAT_*` constants.
    pub nat_type: String,
}

impl NatInfo {
    fn unknown() -> Self {
        Self { public_ip: None, public_port: None, nat_type: NAT_UNKNOWN.to_string() }
    }
}

/// A monotonic, process-unique 96-bit transaction id (uniqueness is all STUN
/// needs here; not used for security).
fn transaction_id() -> [u8; 12] {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut id = [0u8; 12];
    id[..8].copy_from_slice(&t.to_be_bytes());
    id[8..].copy_from_slice(&(n as u32).to_be_bytes());
    id
}

/// Encode a STUN Binding Request (no attributes).
fn binding_request(txid: &[u8; 12]) -> [u8; 20] {
    let mut buf = [0u8; 20];
    buf[0..2].copy_from_slice(&0x0001u16.to_be_bytes()); // Binding Request
    buf[2..4].copy_from_slice(&0u16.to_be_bytes()); // length = 0
    buf[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf[8..20].copy_from_slice(txid);
    buf
}

/// Encode an IPv4 XOR-MAPPED-ADDRESS attribute value.
fn xor_mapped_value(addr: SocketAddr) -> Option<[u8; 8]> {
    let SocketAddr::V4(v4) = addr else { return None };
    let xport = v4.port() ^ ((MAGIC_COOKIE >> 16) as u16);
    let xaddr = u32::from(*v4.ip()) ^ MAGIC_COOKIE;
    let mut v = [0u8; 8];
    v[0] = 0x00;
    v[1] = 0x01; // family IPv4
    v[2..4].copy_from_slice(&xport.to_be_bytes());
    v[4..8].copy_from_slice(&xaddr.to_be_bytes());
    Some(v)
}

/// Build a STUN Binding **Success Response** carrying an XOR-MAPPED-ADDRESS.
/// Exposed for tests (and any in-process mock STUN server).
pub fn binding_success(txid: &[u8; 12], mapped: SocketAddr) -> Vec<u8> {
    let val = xor_mapped_value(mapped).expect("ipv4");
    let mut buf = Vec::with_capacity(32);
    buf.extend_from_slice(&0x0101u16.to_be_bytes()); // Binding Success Response
    buf.extend_from_slice(&12u16.to_be_bytes()); // attr section length (4 hdr + 8 val)
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(txid);
    buf.extend_from_slice(&0x0020u16.to_be_bytes()); // XOR-MAPPED-ADDRESS
    buf.extend_from_slice(&8u16.to_be_bytes());
    buf.extend_from_slice(&val);
    buf
}

/// Parse the reflexive address from a STUN Binding Success Response (matching
/// `txid`). Handles XOR-MAPPED-ADDRESS and plain MAPPED-ADDRESS (IPv4).
fn parse_response(resp: &[u8], txid: &[u8; 12]) -> Option<SocketAddr> {
    if resp.len() < 20 {
        return None;
    }
    if u16::from_be_bytes([resp[0], resp[1]]) != 0x0101 {
        return None; // not a Binding Success Response
    }
    if &resp[8..20] != txid {
        return None; // transaction id mismatch
    }
    let mut i = 20;
    while i + 4 <= resp.len() {
        let atype = u16::from_be_bytes([resp[i], resp[i + 1]]);
        let alen = u16::from_be_bytes([resp[i + 2], resp[i + 3]]) as usize;
        let vstart = i + 4;
        if vstart + alen > resp.len() {
            break;
        }
        let v = &resp[vstart..vstart + alen];
        match atype {
            0x0020 => {
                if let Some(sa) = parse_addr(v, true) {
                    return Some(sa);
                }
            }
            0x0001 => {
                if let Some(sa) = parse_addr(v, false) {
                    return Some(sa);
                }
            }
            _ => {}
        }
        // attributes are padded to a 4-byte boundary
        i = vstart + alen + ((4 - (alen % 4)) % 4);
    }
    None
}

/// Parse a (XOR-)MAPPED-ADDRESS attribute value (IPv4 only).
fn parse_addr(v: &[u8], xored: bool) -> Option<SocketAddr> {
    if v.len() < 8 || v[1] != 0x01 {
        return None; // need IPv4
    }
    let raw_port = u16::from_be_bytes([v[2], v[3]]);
    let raw_addr = u32::from_be_bytes([v[4], v[5], v[6], v[7]]);
    let (port, addr) = if xored {
        (raw_port ^ ((MAGIC_COOKIE >> 16) as u16), raw_addr ^ MAGIC_COOKIE)
    } else {
        (raw_port, raw_addr)
    };
    Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(addr)), port))
}

/// Send a Binding Request to `server` from `sock` and parse the reflexive addr.
async fn stun_binding(sock: &UdpSocket, server: &str, timeout: Duration) -> Option<SocketAddr> {
    let id = transaction_id();
    let req = binding_request(&id);
    let dst = tokio::net::lookup_host(server).await.ok()?.next()?;
    sock.send_to(&req, dst).await.ok()?;
    let mut buf = [0u8; 512];
    let (n, _) = tokio::time::timeout(timeout, sock.recv_from(&mut buf)).await.ok()?.ok()?;
    parse_response(&buf[..n], &id)
}

/// Best-effort host egress IP (the source IP used to reach the internet).
async fn local_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    s.connect("8.8.8.8:80").await.ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

/// Probe the public IP and NAT type using the given STUN servers (`host:port`).
/// Never fails — returns `nat_type = "blocked"/"unknown"` when nothing answers.
pub async fn probe(stun_servers: &[String], timeout: Duration) -> NatInfo {
    if stun_servers.is_empty() {
        return NatInfo::unknown();
    }
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await else {
        return NatInfo::unknown();
    };

    let local = local_ip().await;
    let r1 = stun_binding(&sock, &stun_servers[0], timeout).await;
    // Use the SAME socket against a second server for the mapping test.
    let r2 = if stun_servers.len() > 1 {
        stun_binding(&sock, &stun_servers[1], timeout).await
    } else {
        None
    };

    let reflexive = r1.or(r2);
    let nat_type = match (r1, r2) {
        (None, None) => NAT_BLOCKED,
        (Some(a), None) | (None, Some(a)) => {
            if local == Some(a.ip()) { NAT_OPEN } else { NAT_NAT }
        }
        (Some(a1), Some(a2)) => {
            if local == Some(a1.ip()) {
                NAT_OPEN
            } else if a1 == a2 {
                NAT_CONE
            } else {
                NAT_SYMMETRIC
            }
        }
    };

    NatInfo {
        public_ip: reflexive.map(|a| a.ip().to_string()),
        public_port: reflexive.map(|a| a.port()),
        nat_type: nat_type.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal in-process STUN server: replies to every Binding Request with a
    /// Success Response carrying `mapped` as the XOR-MAPPED-ADDRESS. Returns its
    /// `host:port`.
    async fn mock_stun(mapped: SocketAddr) -> String {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = sock.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                let Ok((n, from)) = sock.recv_from(&mut buf).await else { break };
                if n < 20 {
                    continue;
                }
                let txid: [u8; 12] = buf[8..20].try_into().unwrap();
                let resp = binding_success(&txid, mapped);
                let _ = sock.send_to(&resp, from).await;
            }
        });
        addr.to_string()
    }

    #[test]
    fn xor_mapped_round_trips() {
        let addr: SocketAddr = "203.0.113.5:40000".parse().unwrap();
        let txid = [7u8; 12];
        let resp = binding_success(&txid, addr);
        assert_eq!(parse_response(&resp, &txid), Some(addr));
        // wrong transaction id is rejected
        assert_eq!(parse_response(&resp, &[9u8; 12]), None);
    }

    #[tokio::test]
    async fn same_mapping_from_both_servers_is_cone() {
        // Both STUN servers report the SAME reflexive address → cone.
        let mapped: SocketAddr = "203.0.113.5:55000".parse().unwrap();
        let s1 = mock_stun(mapped).await;
        let s2 = mock_stun(mapped).await;
        let info = probe(&[s1, s2], Duration::from_millis(500)).await;
        assert_eq!(info.nat_type, NAT_CONE);
        assert_eq!(info.public_ip.as_deref(), Some("203.0.113.5"));
        assert_eq!(info.public_port, Some(55000));
    }

    #[tokio::test]
    async fn different_mapping_per_server_is_symmetric() {
        // Different reflexive port per server → endpoint-dependent → symmetric.
        let s1 = mock_stun("203.0.113.5:55000".parse().unwrap()).await;
        let s2 = mock_stun("203.0.113.5:55001".parse().unwrap()).await;
        let info = probe(&[s1, s2], Duration::from_millis(500)).await;
        assert_eq!(info.nat_type, NAT_SYMMETRIC);
    }

    #[tokio::test]
    async fn no_stun_server_is_blocked() {
        // Nothing listening on these ports → blocked (no panic, graceful).
        let info = probe(
            &["127.0.0.1:1".to_string(), "127.0.0.1:2".to_string()],
            Duration::from_millis(200),
        )
        .await;
        assert_eq!(info.nat_type, NAT_BLOCKED);
        assert!(info.public_ip.is_none());
    }

    #[tokio::test]
    async fn single_server_behind_nat_is_undetermined() {
        let s1 = mock_stun("203.0.113.9:6000".parse().unwrap()).await;
        let info = probe(&[s1], Duration::from_millis(500)).await;
        // Only one server → mapping test impossible → generic "nat".
        assert_eq!(info.nat_type, NAT_NAT);
        assert_eq!(info.public_ip.as_deref(), Some("203.0.113.9"));
    }
}
