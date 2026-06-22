//! Call reservations — bridges `AllocateCall` (gRPC) and the SIP INVITE arrival.
//!
//! When the Edge calls `AllocateCall`, the Worker optimistically reserves a slot
//! (increments `active_calls`) and records the `call_id` here with a timestamp.
//! When the matching internal INVITE arrives, `WorkerCallRouter::resolve` *claims*
//! the reservation instead of incrementing again — so a reserved call is counted
//! exactly once. A reaper drops reservations whose INVITE never arrived (TTL),
//! releasing the slot. Calls with no prior reservation (direct INVITE) fall back
//! to incrementing on arrival as before.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use tracing::{debug, warn};

/// Unix-millis now (reservations time out by wall clock).
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Tracks reserved-but-not-yet-arrived calls. Cloneable handle.
#[derive(Clone)]
pub struct CallReservations {
    inner: Arc<Mutex<HashMap<String, i64>>>, // call_id -> reserved_at_ms
    active_calls: Arc<AtomicU32>,
    /// Reservations older than this (ms) are reaped, releasing their slot.
    ttl_ms: i64,
}

impl CallReservations {
    pub fn new(active_calls: Arc<AtomicU32>, ttl_ms: i64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            active_calls,
            ttl_ms,
        }
    }

    /// Reserve a slot for `call_id`: increment `active_calls` and record the
    /// pending reservation. Idempotent — re-reserving the same id is a no-op
    /// (no double increment), so a retried AllocateCall is safe.
    pub fn reserve(&self, call_id: &str) {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if map.contains_key(call_id) {
            return;
        }
        map.insert(call_id.to_string(), now_ms());
        let n = self.active_calls.fetch_add(1, Ordering::Relaxed) + 1;
        debug!(call_id, active_calls = n, "reserved call slot");
    }

    /// Try to claim a reservation when the INVITE arrives. Returns true if a
    /// reservation existed (slot already counted — caller must NOT increment);
    /// false if none (caller increments as usual).
    pub fn claim(&self, call_id: &str) -> bool {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let claimed = map.remove(call_id).is_some();
        if claimed {
            debug!(call_id, "claimed reservation on INVITE arrival");
        }
        claimed
    }

    /// Remove expired reservations, releasing their slots. Returns count reaped.
    pub fn reap_expired(&self) -> u32 {
        let cutoff = now_ms() - self.ttl_ms;
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let expired: Vec<String> = map
            .iter()
            .filter(|(_, at)| **at < cutoff)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            map.remove(id);
            self.active_calls.fetch_sub(1, Ordering::Relaxed);
            warn!(call_id = %id, "reservation expired without INVITE — released slot");
        }
        expired.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (CallReservations, Arc<AtomicU32>) {
        let active = Arc::new(AtomicU32::new(0));
        (CallReservations::new(Arc::clone(&active), 10_000), active)
    }

    #[test]
    fn reserve_increments_once_and_claim_does_not_double_count() {
        let (res, active) = setup();
        res.reserve("c1");
        assert_eq!(active.load(Ordering::Relaxed), 1, "reserve increments");

        // Re-reserving the same id (retried AllocateCall) must not double-count.
        res.reserve("c1");
        assert_eq!(active.load(Ordering::Relaxed), 1, "re-reserve is a no-op");

        // INVITE arrives → claim. Slot already counted, so caller must NOT add.
        assert!(res.claim("c1"), "claim finds the reservation");
        assert_eq!(active.load(Ordering::Relaxed), 1, "claim doesn't change count");

        // Second claim (no reservation left) → false, caller increments itself.
        assert!(!res.claim("c1"), "already-claimed reservation is gone");
    }

    #[test]
    fn claim_unknown_call_returns_false() {
        let (res, _active) = setup();
        assert!(!res.claim("never-reserved"), "no reservation → direct-INVITE path");
    }

    #[test]
    fn reap_releases_expired_but_keeps_fresh() {
        // 1ms TTL: after a short sleep, reservations are stale.
        let active = Arc::new(AtomicU32::new(0));
        let res = CallReservations::new(Arc::clone(&active), 1);
        res.reserve("old1");
        res.reserve("old2");
        assert_eq!(active.load(Ordering::Relaxed), 2);

        std::thread::sleep(std::time::Duration::from_millis(5));
        let reaped = res.reap_expired();
        assert_eq!(reaped, 2, "both stale reservations reaped");
        assert_eq!(active.load(Ordering::Relaxed), 0, "slots released");
        assert!(!res.claim("old1"), "reaped reservation is gone");
    }

    #[test]
    fn reap_keeps_fresh_reservations() {
        let active = Arc::new(AtomicU32::new(0));
        // Long TTL: nothing should be reaped.
        let res = CallReservations::new(Arc::clone(&active), 60_000);
        res.reserve("fresh");
        assert_eq!(res.reap_expired(), 0, "fresh reservation not reaped");
        assert_eq!(active.load(Ordering::Relaxed), 1);
        assert!(res.claim("fresh"), "fresh reservation still claimable");
    }
}
