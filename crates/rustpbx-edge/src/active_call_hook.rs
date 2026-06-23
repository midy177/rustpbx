//! `EdgeActiveCallHook` — keeps the edge's `active_calls` gauge live.
//!
//! The counter is incremented optimistically in `EdgeCallRouter::resolve` when a
//! call is accepted for dispatch. This hook decrements it on
//! `on_record_completed`, which the `CallModule` fires for every call that
//! produced a CDR (answered, rejected, or failed after alerting). Mirrors the
//! worker's `ActiveCallTrackerHook`; the CDR is used only for this gauge on the
//! edge — the authoritative CDR is reported by the worker, not the edge.

use anyhow::Result;
use async_trait::async_trait;
use rustpbx::callrecord::{CallRecord, CallRecordHook};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::debug;

pub struct EdgeActiveCallHook {
    pub active_calls: Arc<AtomicU32>,
}

#[async_trait]
impl CallRecordHook for EdgeActiveCallHook {
    async fn on_record_completed(&self, record: &mut CallRecord) -> Result<()> {
        // Saturating decrement: never wrap below zero if a stray CDR arrives.
        let _ = self.active_calls.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            (v > 0).then(|| v - 1)
        });
        debug!(
            call_id = %record.call_id,
            active_calls = self.active_calls.load(Ordering::Relaxed),
            "edge call completed — decremented gauge"
        );
        Ok(())
    }
}
