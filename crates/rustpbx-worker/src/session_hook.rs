//! `ActiveCallTrackerHook` — tracks the active call counter via the CDR hook
//! pipeline instead of `CallSessionHook` (which is `pub(crate)` in the main
//! crate and inaccessible from external crates).
//!
//! The counter is incremented optimistically in `WorkerCallRouter::resolve`
//! when an internal call is accepted. This hook decrements on
//! `on_record_completed` — which fires for every call that generated a CDR
//! (answered, rejected, or failed after alerting).

use crate::metrics::WorkerMetrics;
use anyhow::Result;
use async_trait::async_trait;
use rustpbx::callrecord::{CallRecord, CallRecordHook};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tracing::debug;

pub struct ActiveCallTrackerHook {
    pub active_calls: Arc<AtomicU32>,
    pub metrics: Arc<WorkerMetrics>,
}

#[async_trait]
impl CallRecordHook for ActiveCallTrackerHook {
    async fn on_record_completed(&self, record: &mut CallRecord) -> Result<()> {
        let prev = self.active_calls.fetch_sub(1, Ordering::Relaxed);
        self.metrics.call_ended();
        let current = prev.saturating_sub(1);
        debug!(
            call_id = %record.call_id,
            active_calls = current,
            "call record completed — decremented counter"
        );
        Ok(())
    }
}
