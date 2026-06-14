//! Addon integration for the distributed Worker.
//!
//! Preserves call-processing addon capabilities (Prometheus metrics, etc.)
//! without requiring the full monolith `AppState`. Each addon hook is
//! collected here and wired into the SIP server / call record manager by
//! `main.rs`.
//!
//! Design note: the main crate's `AddonRegistry` requires `AppState` /
//! `CoreContext` for initialization, which the Worker doesn't have. Instead,
//! we instantiate addon hooks directly — this works because the hooks we need
//! (`MetricsCallRecordHook`) are stateless and use the global `metrics`
//! recorder that the Worker installs via `start_metrics_server`.

use rustpbx::callrecord::CallRecordHook;

/// Collect CDR hooks from addons that provide them.
///
/// These hooks run alongside the Worker's own `GrpcCdrHook` (which uploads
/// CDRs to the Control Plane). The order matters: metrics hooks should run
/// first (fast, non-blocking), then the gRPC upload hook.
///
/// Currently provides:
/// - `MetricsCallRecordHook` (observability) — `rustpbx_calls_total`,
///   `rustpbx_call_duration_seconds`, `rustpbx_call_talk_time_seconds`, etc.
///   Uses the global Prometheus recorder installed by `start_metrics_server`.
pub fn collect_addon_cdr_hooks() -> Vec<Box<dyn CallRecordHook>> {
    let hooks: Vec<Box<dyn CallRecordHook>> = vec![
        // ── Observability: call-level Prometheus metrics ───────────────────
        // Records counters/histograms on every completed call. Writes to the
        // global `metrics` recorder, which the Worker serves at `metrics_addr`.
        Box::new(rustpbx::addons::observability::MetricsCallRecordHook),
    ];
    hooks
}

/// Install addon globals that have process-wide side effects.
///
/// Called once at startup, before the SIP server binds. Currently a no-op
/// because the Worker installs its own Prometheus recorder in
/// `start_metrics_server` (which `MetricsCallRecordHook` writes to via the
/// `metrics` crate's global handle).
pub fn init_addon_globals() {
    // Reserved for future addons that need global initialization
    // (e.g., OpenTelemetry SDK if `addon-telemetry` is enabled).
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_returns_metrics_hook() {
        let hooks = collect_addon_cdr_hooks();
        assert!(!hooks.is_empty(), "should include at least MetricsCallRecordHook");
    }

    #[test]
    fn init_globals_does_not_panic() {
        init_addon_globals();
    }
}
