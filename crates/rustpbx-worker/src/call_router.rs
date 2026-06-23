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

use crate::headers::encode_headers;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rsipstack::dialog::invitation::InviteOption;
use rsipstack::sip::Uri;
use rsipstack::sip::Transport;
use rsipstack::sip::prelude::HeadersExt;
use rustpbx::call::{
    DialDirection, DialStrategy, Dialplan, Location, RouteInvite,
    RoutingState, TransactionCookie, user::SipUser,
};
use rustpbx::call::cookie::{TenantId, TrunkContext};
use rustpbx::config::{MediaProxyMode, RouteResult, RtpConfig};
use rustpbx::proxy::call::{CallRouter, RouteError};
use rustpbx::proxy::data::ProxyDataContext;
use rustpbx::proxy::routing::matcher::{RouteResourceLookup, RouteTrace, match_invite_with_trace};
use rustpbx_core::internal::{DialStrategyKind, InternalCallContext, InternalDirection, RouteAction};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

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
        _route_invite: Box<dyn RouteInvite>,
        caller: &SipUser,
        cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        // InternalPeerModule stores InternalCallContext in the cookie.
        if let Some(internal_ctx) = cookie.get_extension::<InternalCallContext>() {
            return self.resolve_internal(original, internal_ctx, caller, cookie).await;
        }

        // Non-internal call — a local extension dialing out. If an Edge is
        // configured, route-match to pick an egress trunk and forward the
        // INVITE to the Edge (which applies trunk credentials → carrier).
        if self.edge_sip_addr.is_some() {
            return self
                .resolve_outbound_origination(original, caller, cookie)
                .await;
        }

        // No Edge configured — local-to-local routing is out of scope here.
        warn!("worker received non-internal call but no edge_sip_addr configured — rejecting");
        Err(RouteError::from((
            anyhow!("worker-only mode: outbound origination requires edge_sip_addr"),
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

        let mut dialplan = Dialplan::new(session_id.clone(), original.clone(), dial_direction)
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

        let callee_uri = resolve_callee_uri(original)
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
            if trunks.is_empty() { None } else { Some(&trunks) },
            if routes.is_empty() { None } else { Some(&routes) },
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
        dialplan.media.proxy_mode = MediaProxyMode::All;
        dialplan.media.external_ip = self.rtp_config.external_ip.clone();
        dialplan.media.rtp_start_port = self.rtp_config.start_port;
        dialplan.media.rtp_end_port = self.rtp_config.end_port;
        // Let SIP failure codes from the Edge/carrier pass back to the extension.
        dialplan = dialplan.with_passthrough_failure(true);

        let prev = self.active_calls.fetch_add(1, Ordering::Relaxed);
        self.metrics.call_started();
        debug!(active_calls = prev + 1, "worker accepted outbound origination");

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

