//! Shared STUN defaults.
//!
//! A single source of truth for the default public STUN servers, used by:
//! - the Edge / Worker node-local fallback (`config.default_stun_servers`), and
//! - the Control Plane platform settings, so `GET /platform/settings` shows
//!   these when the superadmin hasn't configured a custom list.
//!
//! Order matters: [`probe`](../../rustpbx_netprobe/fn.probe.html) tries each
//! server in turn and uses the first that answers. We list an RFC 5780-capable
//! server first so the full four-type NAT classification can succeed when it's
//! reachable, with two stable Google servers after it as a reliable fallback
//! (public IP + cone/symmetric) if the first is down.

/// Default public STUN servers, ordered by preference.
pub const DEFAULT_STUN_SERVERS: &[&str] = &[
    // RFC 5780 capable (CHANGE-REQUEST / CHANGED-ADDRESS) → enables full Cone /
    // Restricted-Cone / Port-Restricted-Cone / Symmetric classification.
    // Public STUNTMAN instance; historically intermittent, hence the fallbacks.
    "stunserver.stunprotocol.org:3478",
    // Stable Google servers → public IP + cone/symmetric when the first is down
    // (Google STUN does not support CHANGE-REQUEST).
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
];

/// Owned `String` copies of [`DEFAULT_STUN_SERVERS`] (for config defaults that
/// need a `Vec<String>`).
pub fn default_stun_servers() -> Vec<String> {
    DEFAULT_STUN_SERVERS.iter().map(|s| s.to_string()).collect()
}
