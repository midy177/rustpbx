//! STUN-based public-IP and NAT-type detection (RFC 3489 / RFC 5780).
//!
//! Given STUN servers, [`probe`] learns the node's server-reflexive (public)
//! address and classifies its NAT. With a STUN server that has two IPs and
//! supports `CHANGE-REQUEST` (advertising `CHANGED-ADDRESS`/`OTHER-ADDRESS`),
//! it distinguishes the four classic types by combining the **mapping** test
//! (is the mapping endpoint-independent?) with **filtering** tests (does the
//! NAT let a differently-sourced reply through?):
//!
//! - `open`                 — no NAT (reflexive == local).
//! - `full_cone`            — any external host:port can reach the mapping.
//! - `restricted_cone`      — only previously-contacted IPs (any port).
//! - `port_restricted_cone` — only previously-contacted IP:port.
//! - `symmetric`            — different mapping per destination.
//! - `firewall`             — no NAT but UDP filtered (symmetric firewall).
//! - `blocked`              — no STUN reachable.
//!
//! With a basic STUN server (e.g. Google's, no `CHANGE-REQUEST`), it falls back
//! to the mapping-only test: `cone` (endpoint-independent) vs `symmetric`.

pub mod health;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;

const MAGIC_COOKIE: u32 = 0x2112_A442;

// CHANGE-REQUEST flags.
const CHANGE_IP: u32 = 0x04;
const CHANGE_PORT: u32 = 0x02;

pub const NAT_OPEN: &str = "open";
pub const NAT_FULL_CONE: &str = "full_cone";
pub const NAT_RESTRICTED_CONE: &str = "restricted_cone";
pub const NAT_PORT_RESTRICTED_CONE: &str = "port_restricted_cone";
pub const NAT_SYMMETRIC: &str = "symmetric";
pub const NAT_FIREWALL: &str = "firewall";
/// Endpoint-independent mapping but filtering couldn't be tested (basic STUN).
pub const NAT_CONE: &str = "cone";
/// Behind a NAT but only one basic STUN server answered — type undetermined.
pub const NAT_NAT: &str = "nat";
pub const NAT_BLOCKED: &str = "blocked";
pub const NAT_UNKNOWN: &str = "unknown";

/// Result of a NAT probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatInfo {
    pub public_ip: Option<String>,
    pub public_port: Option<u16>,
    pub nat_type: String,
}

impl NatInfo {
    fn unknown() -> Self {
        Self { public_ip: None, public_port: None, nat_type: NAT_UNKNOWN.to_string() }
    }
    fn from(mapped: Option<SocketAddr>, nat_type: &str) -> Self {
        Self {
            public_ip: mapped.map(|a| a.ip().to_string()),
            public_port: mapped.map(|a| a.port()),
            nat_type: nat_type.to_string(),
        }
    }
}

// ── STUN message codec ─────────────────────────────────────────────────────────

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

/// Encode a Binding Request, optionally with a CHANGE-REQUEST attribute.
fn binding_request(txid: &[u8; 12], change_ip: bool, change_port: bool) -> Vec<u8> {
    let with_change = change_ip || change_port;
    let attr_len: u16 = if with_change { 8 } else { 0 };
    let mut buf = Vec::with_capacity(20 + attr_len as usize);
    buf.extend_from_slice(&0x0001u16.to_be_bytes()); // Binding Request
    buf.extend_from_slice(&attr_len.to_be_bytes());
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(txid);
    if with_change {
        let mut flags = 0u32;
        if change_ip {
            flags |= CHANGE_IP;
        }
        if change_port {
            flags |= CHANGE_PORT;
        }
        buf.extend_from_slice(&0x0003u16.to_be_bytes()); // CHANGE-REQUEST
        buf.extend_from_slice(&4u16.to_be_bytes());
        buf.extend_from_slice(&flags.to_be_bytes());
    }
    buf
}

/// Build a Binding Success Response with an XOR-MAPPED-ADDRESS and optionally a
/// CHANGED-ADDRESS (the server's alternate IP:port). Exposed for tests.
pub fn binding_success(txid: &[u8; 12], mapped: SocketAddr, changed: Option<SocketAddr>) -> Vec<u8> {
    let mut attrs = Vec::new();
    // XOR-MAPPED-ADDRESS
    if let SocketAddr::V4(v4) = mapped {
        let xport = v4.port() ^ ((MAGIC_COOKIE >> 16) as u16);
        let xaddr = u32::from(*v4.ip()) ^ MAGIC_COOKIE;
        attrs.extend_from_slice(&0x0020u16.to_be_bytes());
        attrs.extend_from_slice(&8u16.to_be_bytes());
        attrs.extend_from_slice(&[0x00, 0x01]);
        attrs.extend_from_slice(&xport.to_be_bytes());
        attrs.extend_from_slice(&xaddr.to_be_bytes());
    }
    // CHANGED-ADDRESS (plain, not XORed)
    if let Some(SocketAddr::V4(v4)) = changed {
        attrs.extend_from_slice(&0x0005u16.to_be_bytes());
        attrs.extend_from_slice(&8u16.to_be_bytes());
        attrs.extend_from_slice(&[0x00, 0x01]);
        attrs.extend_from_slice(&v4.port().to_be_bytes());
        attrs.extend_from_slice(&u32::from(*v4.ip()).to_be_bytes());
    }
    let mut buf = Vec::with_capacity(20 + attrs.len());
    buf.extend_from_slice(&0x0101u16.to_be_bytes()); // Success Response
    buf.extend_from_slice(&(attrs.len() as u16).to_be_bytes());
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(txid);
    buf.extend_from_slice(&attrs);
    buf
}

/// Parsed reflexive + alternate address from a Binding Success Response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StunResp {
    mapped: SocketAddr,
    /// CHANGED-ADDRESS / OTHER-ADDRESS — the server's alternate IP:port.
    other: Option<SocketAddr>,
}

fn parse_response(resp: &[u8], txid: &[u8; 12]) -> Option<StunResp> {
    if resp.len() < 20 || u16::from_be_bytes([resp[0], resp[1]]) != 0x0101 {
        return None;
    }
    if &resp[8..20] != txid {
        return None;
    }
    let mut mapped = None;
    let mut other = None;
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
            0x0020 => mapped = mapped.or_else(|| parse_addr(v, true)), // XOR-MAPPED-ADDRESS
            0x0001 => mapped = mapped.or_else(|| parse_addr(v, false)), // MAPPED-ADDRESS
            0x0005 | 0x802c => other = other.or_else(|| parse_addr(v, false)), // CHANGED / OTHER-ADDRESS
            _ => {}
        }
        i = vstart + alen + ((4 - (alen % 4)) % 4);
    }
    mapped.map(|mapped| StunResp { mapped, other })
}

fn parse_addr(v: &[u8], xored: bool) -> Option<SocketAddr> {
    if v.len() < 8 || v[1] != 0x01 {
        return None; // IPv4 only
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

// ── STUN I/O ───────────────────────────────────────────────────────────────────

/// Send a Binding Request to `dst` and parse the response. Used for all tests.
async fn stun_test(
    sock: &UdpSocket,
    dst: SocketAddr,
    change_ip: bool,
    change_port: bool,
    timeout: Duration,
) -> Option<StunResp> {
    let id = transaction_id();
    let req = binding_request(&id, change_ip, change_port);
    sock.send_to(&req, dst).await.ok()?;
    let mut buf = [0u8; 512];
    // Retry recv until we see the response matching our txid or time out — a
    // CHANGE-REQUEST reply arrives from a different source address.
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let (n, _from) = tokio::time::timeout(remaining, sock.recv_from(&mut buf)).await.ok()?.ok()?;
        if let Some(r) = parse_response(&buf[..n], &id) {
            return Some(r);
        }
    }
}

async fn local_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    s.connect("8.8.8.8:80").await.ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

// ── Classification ─────────────────────────────────────────────────────────────

/// Outcomes of the RFC 3489 tests. Pure decision function so the full
/// four-type tree is unit-testable without a real NAT.
struct Outcomes {
    behind_nat: bool,
    /// Test 2 (change IP+port) got a reply.
    change_ip_port_reply: bool,
    /// Mapping to the alternate server IP equals the primary mapping.
    mapping_consistent: bool,
    /// Test 3 (change port only) got a reply.
    change_port_reply: bool,
}

fn classify(o: &Outcomes) -> &'static str {
    if !o.behind_nat {
        // No NAT: if a differently-sourced reply gets through, fully open;
        // otherwise a symmetric UDP firewall.
        return if o.change_ip_port_reply { NAT_OPEN } else { NAT_FIREWALL };
    }
    if o.change_ip_port_reply {
        return NAT_FULL_CONE; // any external host:port reaches the mapping
    }
    if !o.mapping_consistent {
        return NAT_SYMMETRIC; // endpoint-dependent mapping
    }
    if o.change_port_reply {
        NAT_RESTRICTED_CONE // address-restricted (any port from a known IP)
    } else {
        NAT_PORT_RESTRICTED_CONE
    }
}

// ── Probe ──────────────────────────────────────────────────────────────────────

/// Probe public IP + NAT type. Never fails.
pub async fn probe(stun_servers: &[String], timeout: Duration) -> NatInfo {
    if stun_servers.is_empty() {
        return NatInfo::unknown();
    }
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await else {
        return NatInfo::unknown();
    };
    let local = local_ip().await;

    // Test 1: plain Binding Request. Try each server in order and use the first
    // that answers as the primary. This makes an unstable RFC-5780 server
    // listed first non-fatal — if it's down we fall through to the next entry
    // and still learn the mapped address instead of reporting "blocked".
    let mut primary_addr: Option<SocketAddr> = None;
    let mut primary_idx = 0usize;
    let mut t1 = None;
    for (i, s) in stun_servers.iter().enumerate() {
        let Some(dst) = resolve(s).await else { continue };
        if let Some(r) = stun_test(&sock, dst, false, false, timeout).await {
            primary_addr = Some(dst);
            primary_idx = i;
            t1 = Some(r);
            break;
        }
    }
    let Some(primary) = primary_addr else {
        return NatInfo::from(None, NAT_BLOCKED); // no server answered at all
    };
    let t1 = t1.unwrap();
    let behind_nat = local != Some(t1.mapped.ip());

    // Without a server-advertised alternate address we can't run the filtering
    // tests → fall back to the mapping-only test (cone vs symmetric).
    let Some(other) = t1.other else {
        return fallback_mapping(&sock, stun_servers, primary_idx, t1.mapped, behind_nat, timeout)
            .await;
    };

    // Test 2: ask the server to reply from a different IP *and* port.
    let change_ip_port_reply = stun_test(&sock, primary, true, true, timeout).await.is_some();

    // Test 1b: query the alternate server IP (same port) to check the mapping.
    let alt = SocketAddr::new(other.ip(), primary.port());
    let mapping_consistent = stun_test(&sock, alt, false, false, timeout)
        .await
        .map(|r| r.mapped == t1.mapped)
        .unwrap_or(true);

    // Test 3: ask the server to reply from the same IP, different port.
    let change_port_reply = stun_test(&sock, primary, false, true, timeout).await.is_some();

    let nat = classify(&Outcomes {
        behind_nat,
        change_ip_port_reply,
        mapping_consistent,
        change_port_reply,
    });
    NatInfo::from(Some(t1.mapped), nat)
}

/// Mapping-only fallback for basic STUN servers (no CHANGE-REQUEST): compare the
/// reflexive mapping seen from a *different* server than the primary.
async fn fallback_mapping(
    sock: &UdpSocket,
    stun_servers: &[String],
    primary_idx: usize,
    mapped1: SocketAddr,
    behind_nat: bool,
    timeout: Duration,
) -> NatInfo {
    if !behind_nat {
        return NatInfo::from(Some(mapped1), NAT_OPEN);
    }
    // Pick the first server that isn't the primary for an independent mapping.
    let other = stun_servers
        .iter()
        .enumerate()
        .find(|(i, _)| *i != primary_idx)
        .map(|(_, s)| s);
    let mapped2 = match other {
        Some(s) => match resolve(s).await {
            Some(dst) => stun_test(sock, dst, false, false, timeout).await.map(|r| r.mapped),
            None => None,
        },
        None => None,
    };
    let nat = match mapped2 {
        Some(m2) if m2 == mapped1 => NAT_CONE, // endpoint-independent mapping
        Some(_) => NAT_SYMMETRIC,              // endpoint-dependent
        None => NAT_NAT,                       // only one usable server → undetermined
    };
    NatInfo::from(Some(mapped1), nat)
}

async fn resolve(server: &str) -> Option<SocketAddr> {
    tokio::net::lookup_host(server).await.ok()?.next()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock STUN server: replies with `mapped` (and optionally a CHANGED-ADDRESS).
    async fn mock_stun(mapped: SocketAddr, changed: Option<SocketAddr>) -> String {
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
                let _ = sock.send_to(&binding_success(&txid, mapped, changed), from).await;
            }
        });
        addr.to_string()
    }

    #[test]
    fn codec_round_trips_mapped_and_changed() {
        let mapped: SocketAddr = "203.0.113.5:40000".parse().unwrap();
        let changed: SocketAddr = "198.51.100.9:3479".parse().unwrap();
        let txid = [7u8; 12];
        let r = parse_response(&binding_success(&txid, mapped, Some(changed)), &txid).unwrap();
        assert_eq!(r.mapped, mapped);
        assert_eq!(r.other, Some(changed));
        assert_eq!(parse_response(&binding_success(&txid, mapped, Some(changed)), &[9u8; 12]), None);
    }

    #[test]
    fn change_request_attribute_is_encoded() {
        let req = binding_request(&[0u8; 12], true, true);
        assert_eq!(req.len(), 28); // 20 header + 8 attr
        assert_eq!(u16::from_be_bytes([req[2], req[3]]), 8); // message length
        assert_eq!(u16::from_be_bytes([req[20], req[21]]), 0x0003); // CHANGE-REQUEST
        let flags = u32::from_be_bytes([req[24], req[25], req[26], req[27]]);
        assert_eq!(flags, CHANGE_IP | CHANGE_PORT);
    }

    // ── Full four-type decision tree (pure, no real NAT needed) ────────────────
    fn t(behind_nat: bool, cipp: bool, consistent: bool, cp: bool) -> &'static str {
        classify(&Outcomes {
            behind_nat,
            change_ip_port_reply: cipp,
            mapping_consistent: consistent,
            change_port_reply: cp,
        })
    }

    #[test]
    fn classifies_all_four_nat_types() {
        // no NAT
        assert_eq!(t(false, true, true, true), NAT_OPEN);
        assert_eq!(t(false, false, true, true), NAT_FIREWALL);
        // behind NAT
        assert_eq!(t(true, true, true, true), NAT_FULL_CONE);
        assert_eq!(t(true, false, false, true), NAT_SYMMETRIC);
        assert_eq!(t(true, false, true, true), NAT_RESTRICTED_CONE);
        assert_eq!(t(true, false, true, false), NAT_PORT_RESTRICTED_CONE);
    }

    // ── Mapping-only fallback (basic STUN, no CHANGED-ADDRESS) ──────────────────
    #[tokio::test]
    async fn fallback_same_mapping_is_cone() {
        let m: SocketAddr = "203.0.113.5:55000".parse().unwrap();
        let s1 = mock_stun(m, None).await;
        let s2 = mock_stun(m, None).await;
        let info = probe(&[s1, s2], Duration::from_millis(500)).await;
        assert_eq!(info.nat_type, NAT_CONE);
        assert_eq!(info.public_ip.as_deref(), Some("203.0.113.5"));
    }

    #[tokio::test]
    async fn fallback_different_mapping_is_symmetric() {
        let s1 = mock_stun("203.0.113.5:55000".parse().unwrap(), None).await;
        let s2 = mock_stun("203.0.113.5:55001".parse().unwrap(), None).await;
        let info = probe(&[s1, s2], Duration::from_millis(500)).await;
        assert_eq!(info.nat_type, NAT_SYMMETRIC);
    }

    #[tokio::test]
    async fn no_stun_is_blocked() {
        let info = probe(
            &["127.0.0.1:1".into(), "127.0.0.1:2".into()],
            Duration::from_millis(200),
        )
        .await;
        assert_eq!(info.nat_type, NAT_BLOCKED);
    }

    #[tokio::test]
    async fn single_basic_server_is_undetermined() {
        let s1 = mock_stun("203.0.113.9:6000".parse().unwrap(), None).await;
        let info = probe(&[s1], Duration::from_millis(500)).await;
        assert_eq!(info.nat_type, NAT_NAT);
    }

    /// A dead/unreachable server listed first must NOT fail the whole probe —
    /// it falls through to the next entry and still returns a mapped address.
    #[tokio::test]
    async fn dead_primary_falls_through_to_next_server() {
        // Binds but never replies (simulates a down STUN server) as servers[0].
        let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let silent_addr = silent.local_addr().unwrap().to_string();
        // Live mock server as servers[1].
        let live = mock_stun("203.0.113.7:30000".parse().unwrap(), None).await;
        let info = probe(&[silent_addr, live], Duration::from_millis(300)).await;
        assert_ne!(info.nat_type, NAT_BLOCKED, "must not report blocked when a later server is reachable");
        assert_eq!(info.public_ip.as_deref(), Some("203.0.113.7"), "fell through and got the mapping");
    }
}
