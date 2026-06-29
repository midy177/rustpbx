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
//! For non-internal calls, outbound origination goes through the Edge when
//! configured; otherwise the Worker builds a local anchored dialplan.

use crate::headers::encode_headers;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rsipstack::dialog::invitation::InviteOption;
use rsipstack::sip::Transport;
use rsipstack::sip::Uri;
use rsipstack::sip::prelude::HeadersExt;
use rustpbx::call::{
    DialDirection, DialStrategy, Dialplan, Location, RouteInvite, RoutingState, TransactionCookie,
    user::SipUser,
};
use rustpbx::config::{DialplanHints, MediaProxyMode, RouteResult, RtpConfig};
use rustpbx::proxy::call::{CallRouter, RouteError};
use rustpbx::proxy::data::ProxyDataContext;
use rustpbx::proxy::routing::matcher::{RouteResourceLookup, RouteTrace, match_invite_with_trace};
use rustpbx_core::internal::{
    DialStrategyKind, InternalCallContext, InternalDirection, RouteAction,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::{debug, info};

pub struct WorkerCallRouter {
    pub data_context: Arc<ProxyDataContext>,
    pub rtp_config: RtpConfig,
    pub routing_state: Arc<RoutingState>,
    /// Active call counter — incremented when a Worker-internal dialplan is
    /// built, decremented by `ActiveCallTrackerHook::on_record_completed`.
    pub active_calls: Arc<AtomicU32>,
    pub metrics: Arc<crate::metrics::WorkerMetrics>,
    /// SIP address of the Edge used for outbound origination (`host:port`).
    /// `None` disables outbound from local extensions (inbound-only worker).
    pub edge_sip_addr: Option<String>,
    /// Pending `AllocateCall` reservations. When an internal INVITE arrives for
    /// a call that was reserved, the slot is *claimed* (already counted) rather
    /// than incremented again.
    pub reservations: crate::reservations::CallReservations,
}

#[async_trait]
impl CallRouter for WorkerCallRouter {
    async fn resolve(
        &self,
        original: &rsipstack::sip::Request,
        route_invite: Box<dyn RouteInvite>,
        caller: &SipUser,
        cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        // InternalPeerModule stores InternalCallContext in the cookie.
        if let Some(internal_ctx) = cookie.get_extension::<InternalCallContext>() {
            return self
                .resolve_internal(original, internal_ctx, caller, cookie)
                .await;
        }

        // Non-internal call — a local extension dialing out. If an Edge is
        // configured, route-match to pick an egress trunk and forward the
        // INVITE to the Edge (which applies trunk credentials → carrier).
        if self.edge_sip_addr.is_some() {
            return self
                .resolve_outbound_origination(original, caller, cookie)
                .await;
        }

        self.resolve_local(original, route_invite, caller, cookie)
            .await
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

        let session_id = original
            .call_id_header()
            .map(|h| h.value().to_string())
            .unwrap_or_else(|_| format!("worker-{}", std::process::id()));

        let action = ctx.action;
        let target_count = ctx.targets.len();
        let app_name = ctx.app_name.clone();
        let queue_name = ctx.queue_name.clone();
        let dialplan = crate::dialplan_resolver::build_internal_dialplan(
            original,
            &ctx,
            caller,
            &self.rtp_config,
            |queue_name| self.load_queue_plan(queue_name),
        )
        .map_err(|e| {
            RouteError::from((
                anyhow!("internal dialplan build failed: {e}"),
                Some(rsipstack::sip::StatusCode::ServerInternalError),
            ))
        })?;
        match action {
            RouteAction::Forward => debug!(targets = target_count, "built forward dialplan"),
            RouteAction::Application => debug!(app = ?app_name, "built application dialplan"),
            RouteAction::Queue => debug!(queue = ?queue_name, "built queue dialplan"),
        }

        // ── Track active calls ────────────────────────────────────────────────
        // If the Edge reserved this call via AllocateCall, the slot is already
        // counted — claim it. Otherwise (direct INVITE), increment now. Either
        // way the CDR hook decrements exactly once on call end.
        if self.reservations.claim(&session_id) {
            self.metrics.call_started();
            debug!(call_id = %session_id, "worker accepted internal call (claimed reservation)");
        } else {
            let prev = self.active_calls.fetch_add(1, Ordering::Relaxed);
            self.metrics.call_started();
            debug!(active_calls = prev + 1, "worker accepted internal call");
        }

        Ok(dialplan)
    }

    /// Worker-only fallback: build a local anchored dialplan when no Edge is
    /// configured. This covers extension-to-extension calls and local
    /// queue/application routes without sending traffic to a carrier edge.
    async fn resolve_local(
        &self,
        original: &rsipstack::sip::Request,
        route_invite: Box<dyn RouteInvite>,
        caller: &SipUser,
        cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        let callee_uri = crate::dialplan_resolver::resolve_callee_uri(original)
            .map_err(|e| RouteError::from((e, None)))?;
        let caller_uri = caller
            .from
            .clone()
            .or_else(|| original.from_header().ok().and_then(|h| h.uri().ok()))
            .ok_or_else(|| {
                RouteError::from((
                    anyhow!("local: cannot resolve caller URI"),
                    Some(rsipstack::sip::StatusCode::BadRequest),
                ))
            })?;
        let session_id = original
            .call_id_header()
            .map(|h| h.value().to_string())
            .unwrap_or_else(|_| format!("worker-local-{}", std::process::id()));

        let option = InviteOption {
            callee: callee_uri.clone(),
            caller: caller_uri.clone(),
            contact: caller_uri.clone(),
            ..Default::default()
        };
        let route_result = route_invite
            .preview_route(option, original, &DialDirection::Internal, cookie)
            .await
            .map_err(|e| {
                RouteError::from((
                    anyhow!("local route preview failed: {e}"),
                    Some(rsipstack::sip::StatusCode::ServerInternalError),
                ))
            })?;

        let mut dialplan = Dialplan::new(session_id, original.clone(), DialDirection::Internal)
            .with_caller(caller_uri)
            .with_route_invite(route_invite)
            .with_passthrough_failure(true);
        crate::dialplan_resolver::apply_worker_media(&mut dialplan, &self.rtp_config);
        dialplan.recording = self
            .data_context
            .config()
            .recording
            .as_ref()
            .map(|p| p.new_recording_config())
            .unwrap_or_default();

        let hints = match route_result {
            RouteResult::Forward(option, hints) => {
                let contact_raw = option.callee.to_string();
                let target = Location {
                    aor: option.callee,
                    destination: option.destination,
                    credential: option.credential,
                    headers: option.headers,
                    contact_raw: Some(contact_raw),
                    ..Default::default()
                };
                dialplan = dialplan.with_targets(DialStrategy::Sequential(vec![target]));
                hints
            }
            RouteResult::Queue { queue, hints, .. } => {
                dialplan = dialplan.with_queue(queue);
                hints
            }
            RouteResult::Application {
                option,
                app_name,
                app_params,
                auto_answer,
                ..
            } => {
                dialplan = dialplan.with_application(app_name, app_params, auto_answer);
                dialplan.routed_headers = option.headers;
                None
            }
            RouteResult::NotHandled(_, hints) => {
                dialplan = dialplan.with_targets(DialStrategy::Sequential(vec![Location {
                    aor: callee_uri,
                    ..Default::default()
                }]));
                hints
            }
            RouteResult::Abort(code, reason) => {
                return Err(RouteError::from((
                    anyhow!(reason.unwrap_or_else(|| "local route aborted".to_string())),
                    Some(code),
                )));
            }
        };
        self.apply_local_hints(&mut dialplan, hints);

        let prev = self.active_calls.fetch_add(1, Ordering::Relaxed);
        self.metrics.call_started();
        debug!(active_calls = prev + 1, "worker accepted local call");
        Ok(dialplan)
    }

    fn apply_local_hints(&self, dialplan: &mut Dialplan, hints: Option<DialplanHints>) {
        let Some(mut hints) = hints else {
            return;
        };
        if let Some(policy) = hints.recording.take() {
            dialplan.recording = policy.new_recording_config();
            dialplan.recording_policy = Some(policy);
        }
        if let Some(enabled) = hints.enable_recording {
            dialplan.recording.enabled = enabled;
        }
        if hints.bypass_media == Some(true) {
            dialplan.media.proxy_mode = MediaProxyMode::Bypass;
        }
        if let Some(media_mode) = hints.media_mode {
            dialplan.media.proxy_mode = media_mode;
        }
        if let Some(video_policy) = hints.video_policy {
            dialplan.media.video_policy = Some(video_policy);
        }
        if let Some(max_duration) = hints.max_duration {
            dialplan.max_call_duration = Some(max_duration);
        }
        if let Some(enable_sipflow) = hints.enable_sipflow {
            dialplan.enable_sipflow = enable_sipflow;
        }
        if hints.disable_ice_servers == Some(true) {
            dialplan.media.ice_servers = None;
        }
        if let Some(ringback) = hints.ringback.take() {
            dialplan.audio_profile = Some(ringback);
        }
        dialplan.extensions = std::mem::take(&mut hints.extensions);
    }

    /// Outbound origination: a local extension dials an external number.
    /// Route-match (Outbound) to select an egress trunk, then forward the call
    /// to the Edge encoded as an internal `X-*` INVITE. The Worker anchors
    /// media (`MediaProxyMode::All`); the Edge bypasses to the carrier.
    async fn resolve_outbound_origination(
        &self,
        original: &rsipstack::sip::Request,
        caller: &SipUser,
        _cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        // edge_sip_addr presence is guaranteed by the caller.
        let edge_addr = self.edge_sip_addr.as_deref().unwrap_or_default();

        let callee_uri = crate::dialplan_resolver::resolve_callee_uri(original)
            .map_err(|e| RouteError::from((e, None)))?;
        let caller_uri = caller
            .from
            .clone()
            .or_else(|| original.from_header().ok().and_then(|h| h.uri().ok()))
            .ok_or_else(|| {
                RouteError::from((
                    anyhow!("outbound: cannot resolve caller URI"),
                    Some(rsipstack::sip::StatusCode::BadRequest),
                ))
            })?;

        // ── Route-match to select the egress trunk ───────────────────────────
        let option = InviteOption {
            callee: callee_uri.clone(),
            caller: caller_uri.clone(),
            contact: caller_uri.clone(),
            ..Default::default()
        };
        let trunks = self.data_context.trunks_snapshot();
        let routes = self.data_context.routes_snapshot();
        let lookup = self.data_context.as_ref() as &dyn RouteResourceLookup;
        let mut trace = RouteTrace::default();
        let result = match_invite_with_trace(
            if trunks.is_empty() {
                None
            } else {
                Some(&trunks)
            },
            if routes.is_empty() {
                None
            } else {
                Some(&routes)
            },
            Some(lookup),
            option,
            original,
            None, // a local extension is not a source trunk
            self.routing_state.clone(),
            &DialDirection::Outbound,
            &mut trace,
        )
        .await
        .map_err(|e| RouteError::from((e, None)))?;

        match result {
            RouteResult::Forward(_, _) => {}
            RouteResult::Abort(code, reason) => {
                return Err(RouteError::from((
                    anyhow!(reason.unwrap_or_else(|| "routing aborted".to_string())),
                    Some(code),
                )));
            }
            _ => {
                return Err(RouteError::from((
                    anyhow!("worker: outbound call did not resolve to a trunk forward"),
                    Some(rsipstack::sip::StatusCode::NotImplemented),
                )));
            }
        }

        let trunk_name = trace.selected_trunk.ok_or_else(|| {
            RouteError::from((
                anyhow!("worker: outbound call matched no egress trunk"),
                Some(rsipstack::sip::StatusCode::NotFound),
            ))
        })?;

        info!(
            trunk = %trunk_name,
            caller = %caller_uri,
            callee = %callee_uri,
            edge = %edge_addr,
            "worker originating outbound call → edge"
        );

        // ── Encode the routing decision for the Edge ──────────────────────────
        let internal_ctx = InternalCallContext {
            edge_id: String::new(),
            tenant_id: None,
            trunk_name,
            trunk_id: None,
            direction: InternalDirection::Outbound,
            action: RouteAction::Forward,
            original_from: caller_uri.to_string(),
            original_to: callee_uri.to_string(),
            targets: vec![callee_uri.to_string()],
            dial_strategy: DialStrategyKind::Sequential,
            app_name: None,
            app_params: None,
            queue_name: None,
            record: false,
            max_duration_secs: None,
        };

        let edge_uri_str = if edge_addr.starts_with("sip:") {
            edge_addr.to_string()
        } else {
            format!("sip:{}", edge_addr)
        };
        let edge_uri = Uri::try_from(edge_uri_str.as_str()).map_err(|e| {
            RouteError::from((
                anyhow!("invalid edge_sip_addr '{}': {}", edge_addr, e),
                None,
            ))
        })?;

        let edge_location = Location {
            aor: edge_uri,
            headers: Some(encode_headers(&internal_ctx)),
            // Reach the Edge over TCP — the Edge↔Worker link is a persistent TCP
            // connection in both directions (mirrors the Edge→Worker path).
            transport: Some(Transport::Tcp),
            ..Default::default()
        };

        let session_id = original
            .call_id_header()
            .map(|h| h.value().to_string())
            .unwrap_or_else(|_| format!("worker-out-{}", std::process::id()));

        let mut dialplan = Dialplan::new(session_id, original.clone(), DialDirection::Outbound)
            .with_caller(caller_uri)
            .with_targets(DialStrategy::Sequential(vec![edge_location]));

        // Worker anchors media: Extension ↔ Worker ↔ Edge(bypass) ↔ Carrier.
        crate::dialplan_resolver::apply_worker_media(&mut dialplan, &self.rtp_config);
        // Let SIP failure codes from the Edge/carrier pass back to the extension.
        dialplan = dialplan.with_passthrough_failure(true);

        let prev = self.active_calls.fetch_add(1, Ordering::Relaxed);
        self.metrics.call_started();
        debug!(
            active_calls = prev + 1,
            "worker accepted outbound origination"
        );

        Ok(dialplan)
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
