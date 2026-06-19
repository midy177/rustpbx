//! `WorkerCallRouter` — Worker-side call router that decodes routing decisions
//! from Edge-encoded `X-*` headers and constructs the appropriate `Dialplan`.
//!
//! When an internal INVITE arrives from the Edge (carrying `X-Route-Action`),
//! the Worker's `InternalPeerModule` injects `InternalCallContext` into the
//! cookie. This router reads it and builds a full-media `Dialplan`:
//!
//! - `Forward`  → `DialStrategy` targeting the specified URIs
//! - `Queue`    → `DialplanFlow::Queue` with the named queue plan
//! - `Application` → `DialplanFlow::Application` for IVR / voicemail
//!
//! For non-internal calls (local extension-to-extension), falls back to the
//! standard routing path — not yet wired, returns `NotImplemented`.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rsipstack::sip::Uri;
use rsipstack::sip::prelude::HeadersExt;
use rustpbx::call::{
    DialDirection, DialStrategy, Dialplan, Location, RouteInvite,
    RoutingState, TransactionCookie, user::SipUser,
};
use rustpbx::call::cookie::{TenantId, TrunkContext};
use rustpbx::config::{MediaProxyMode, RtpConfig};
use rustpbx::proxy::call::{CallRouter, RouteError};
use rustpbx::proxy::data::ProxyDataContext;
use rustpbx_core::internal::{DialStrategyKind, InternalCallContext, InternalDirection, RouteAction};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

pub struct WorkerCallRouter {
    pub data_context: Arc<ProxyDataContext>,
    pub rtp_config: RtpConfig,
    #[allow(dead_code)]
    pub routing_state: Arc<RoutingState>,
    /// Active call counter — incremented when a Worker-internal dialplan is
    /// built, decremented by `ActiveCallTrackerHook::on_record_completed`.
    pub active_calls: Arc<AtomicU32>,
    pub metrics: Arc<crate::metrics::WorkerMetrics>,
}

#[async_trait]
impl CallRouter for WorkerCallRouter {
    async fn resolve(
        &self,
        original: &rsipstack::sip::Request,
        _route_invite: Box<dyn RouteInvite>,
        caller: &SipUser,
        cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        // InternalPeerModule stores InternalCallContext in the cookie.
        if let Some(internal_ctx) = cookie.get_extension::<InternalCallContext>() {
            return self.resolve_internal(original, internal_ctx, caller, cookie).await;
        }

        // Non-internal call — local extension dial.
        // For Phase 3 we do not implement full local routing; reject so the
        // caller gets a clear "not supported in worker-only mode" signal.
        // Future: delegate to DefaultRouteInvite for local resolution.
        warn!("worker received non-internal call — local routing not yet implemented");
        Err(RouteError::from((
            anyhow!("worker-only mode: non-internal routing not supported"),
            Some(rsipstack::sip::StatusCode::NotImplemented),
        )))
    }
}

impl WorkerCallRouter {
    async fn resolve_internal(
        &self,
        original: &rsipstack::sip::Request,
        ctx: InternalCallContext,
        caller: &SipUser,
        _cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        info!(
            edge_id = %ctx.edge_id,
            trunk = %ctx.trunk_name,
            tenant_id = ?ctx.tenant_id,
            action = ctx.action.as_str(),
            "worker received internal call from edge"
        );

        // ── Resolve caller / callee URIs ──────────────────────────────────────
        let callee_uri = resolve_callee_uri(original)
            .map_err(|e| RouteError::from((e, None)))?;
        let caller_uri = caller
            .from
            .clone()
            .or_else(|| {
                original
                    .from_header()
                    .ok()
                    .and_then(|h| h.uri().ok())
            })
            .unwrap_or_else(|| callee_uri.clone());

        let session_id = original
            .call_id_header()
            .map(|h| h.value().to_string())
            .unwrap_or_else(|_| format!("worker-{}", std::process::id()));

        // ── Build the base dialplan ───────────────────────────────────────────
        let dial_direction = match ctx.direction {
            InternalDirection::Inbound => DialDirection::Inbound,
            InternalDirection::Outbound => DialDirection::Outbound,
            InternalDirection::Internal => DialDirection::Internal,
        };

        let mut dialplan = Dialplan::new(session_id, original.clone(), dial_direction)
            .with_caller(caller_uri.clone());

        // ── Build flow based on routing action ────────────────────────────────
        match ctx.action {
            RouteAction::Forward => {
                let locations = self.build_locations(&ctx, &callee_uri);
                if locations.is_empty() {
                    return Err(RouteError::from((
                        anyhow!("forward action with no targets"),
                        Some(rsipstack::sip::StatusCode::ServerInternalError),
                    )));
                }
                let strategy = match ctx.dial_strategy {
                    DialStrategyKind::Parallel => DialStrategy::Parallel(locations),
                    DialStrategyKind::Sequential => DialStrategy::Sequential(locations),
                };
                dialplan = dialplan.with_targets(strategy);
                debug!(targets = ctx.targets.len(), "built forward dialplan");
            }
            RouteAction::Application => {
                let app_name = ctx.app_name.clone().ok_or_else(|| {
                    RouteError::from((
                        anyhow!("application action without app_name"),
                        Some(rsipstack::sip::StatusCode::ServerInternalError),
                    ))
                })?;
                dialplan = dialplan.with_application(app_name, ctx.app_params.clone(), true);
                debug!(app = ?ctx.app_name, "built application dialplan");
            }
            RouteAction::Queue => {
                let queue_name = ctx.queue_name.clone().ok_or_else(|| {
                    RouteError::from((
                        anyhow!("queue action without queue_name"),
                        Some(rsipstack::sip::StatusCode::ServerInternalError),
                    ))
                })?;
                let queue_plan = self
                    .load_queue_plan(&queue_name)
                    .map_err(|e| {
                        RouteError::from((
                            anyhow!("failed to load queue '{}': {}", queue_name, e),
                            Some(rsipstack::sip::StatusCode::ServerInternalError),
                        ))
                    })?;
                dialplan = dialplan.with_queue(queue_plan);
                debug!(queue = %queue_name, "built queue dialplan");
            }
        }

        // ── Configure media (Worker handles full RTP) ─────────────────────────
        dialplan.media.proxy_mode = MediaProxyMode::All;
        dialplan.media.external_ip = self.rtp_config.external_ip.clone();
        dialplan.media.rtp_start_port = self.rtp_config.start_port;
        dialplan.media.rtp_end_port = self.rtp_config.end_port;

        // ── Recording ─────────────────────────────────────────────────────────
        if ctx.record {
            dialplan.recording.enabled = true;
            dialplan.recording.auto_start = true;
        }

        // ── Max duration ──────────────────────────────────────────────────────
        if let Some(secs) = ctx.max_duration_secs {
            dialplan.max_call_duration = Some(std::time::Duration::from_secs(secs));
        }

        // ── Inject trunk context + tenant for CDR attribution ─────────────────
        let trunk_ctx = TrunkContext {
            id: ctx.trunk_id,
            name: ctx.trunk_name.clone(),
            tenant_id: ctx.tenant_id,
            did_numbers: Vec::new(),
        };
        dialplan = dialplan.with_extension(trunk_ctx);
        if let Some(tid) = ctx.tenant_id {
            dialplan = dialplan.with_extension(TenantId(tid));
        }

        // ── Track active calls (optimistic increment; CDR hook decrements on end) ─
        let prev = self.active_calls.fetch_add(1, Ordering::Relaxed);
        self.metrics.call_started();
        debug!(active_calls = prev + 1, "worker accepted internal call");

        Ok(dialplan)
    }

    /// Build `Location` list from the context's targets, falling back to the
    /// original callee URI if no explicit targets were provided.
    fn build_locations(&self, ctx: &InternalCallContext, fallback_callee: &Uri) -> Vec<Location> {
        if ctx.targets.is_empty() {
            return vec![Location {
                aor: fallback_callee.clone(),
                ..Default::default()
            }];
        }
        ctx.targets
            .iter()
            .filter_map(|uri_str| {
                Uri::try_from(uri_str.as_str())
                    .map(|uri| Location {
                        aor: uri,
                        ..Default::default()
                    })
                    .map_err(|e| warn!(uri = %uri_str, error = %e, "invalid target URI, skipping"))
                    .ok()
            })
            .collect()
    }

    /// Resolve a queue plan by name from the data context.
    fn load_queue_plan(&self, name: &str) -> Result<rustpbx::call::QueuePlan> {
        let lookup_ref = if name.trim().chars().all(|c| c.is_ascii_digit()) {
            format!("db-{}", name)
        } else {
            name.to_string()
        };
        let queue_cfg = self
            .data_context
            .resolve_queue_config(&lookup_ref)?
            .ok_or_else(|| anyhow!("queue '{}' not found", name))?;
        queue_cfg.to_queue_plan()
    }
}

/// Extract the callee URI: prefer Request-URI's user part, fall back to To header.
fn resolve_callee_uri(origin: &rsipstack::sip::Request) -> Result<Uri> {
    if origin
        .uri
        .user()
        .map(|user| !user.trim().is_empty())
        .unwrap_or(false)
    {
        return Ok(origin.uri.clone());
    }
    origin
        .to_header()
        .map_err(anyhow::Error::from)?
        .uri()
        .map_err(anyhow::Error::from)
}

