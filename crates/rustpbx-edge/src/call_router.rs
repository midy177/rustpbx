//! `EdgeCallRouter` — the core call dispatch interceptor for the SIP Edge.
//!
//! When the Edge's `CallModule` receives an INVITE, it delegates routing to
//! this router via the `CallRouter` trait. The router:
//!
//! 1. **Inbound path** (carrier trunk → Edge): runs route matching, selects a
//!    Media Worker, encodes the routing decision into `X-*` headers, and
//!    returns a bypass-mode `Dialplan` targeting the Worker's SIP address.
//! 2. **Outbound path** (Worker → Edge → carrier): reads `X-Trunk-Name` from
//!    the internal INVITE, applies the trunk's credentials/DID/header-rewrite,
//!    and returns a bypass `Dialplan` targeting the carrier.
//!
//! Media always flows directly between carrier and Worker (Edge is signaling-only).

use crate::headers::encode_headers;
use crate::worker_selector::WorkerSelector;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rustpbx::call::{
    CallRecordingConfig, DialDirection, DialStrategy, Dialplan, Location, RoutingState,
};
use rustpbx::call::cookie::{TrunkContext, TransactionCookie};
use rustpbx::call::user::SipUser;
use rustpbx::config::{MediaProxyMode, RouteResult};
use rustpbx::proxy::call::{CallRouter, RouteError};
use rustpbx::proxy::data::ProxyDataContext;
use rsipstack::dialog::authenticate::Credential;
use rustpbx_core::internal::{InternalCallContext, InternalDirection, RouteAction};
use rsipstack::dialog::invitation::InviteOption;
use rsipstack::sip::Uri;
use rsipstack::sip::prelude::HeadersExt;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub struct EdgeCallRouter {
    pub worker_selector: Arc<WorkerSelector>,
    pub data_context: Arc<ProxyDataContext>,
    #[allow(dead_code)]
    pub routing_state: Arc<RoutingState>,
    pub edge_id: String,
}

impl EdgeCallRouter {
    pub fn new(
        worker_selector: Arc<WorkerSelector>,
        data_context: Arc<ProxyDataContext>,
        routing_state: Arc<RoutingState>,
        edge_id: String,
    ) -> Self {
        Self { worker_selector, data_context, routing_state, edge_id }
    }
}

#[async_trait]
impl CallRouter for EdgeCallRouter {
    async fn resolve(
        &self,
        original: &rsipstack::sip::Request,
        route_invite: Box<dyn rustpbx::call::RouteInvite>,
        caller: &SipUser,
        cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        if let Some(internal_ctx) = cookie.get_extension::<InternalCallContext>() {
            if matches!(internal_ctx.direction, InternalDirection::Outbound) {
                return self.resolve_outbound(original, &internal_ctx, caller, cookie).await;
            }
        }

        let trunk_ctx = cookie.get_extension::<TrunkContext>();
        if trunk_ctx.is_some() {
            self.resolve_inbound(original, route_invite, caller, cookie).await
        } else {
            Err(RouteError::from((
                anyhow!("edge: no trunk context on INVITE — only trunk-sourced or worker-outbound calls accepted"),
                Some(rsipstack::sip::StatusCode::Forbidden),
            )))
        }
    }
}

impl EdgeCallRouter {
    async fn resolve_outbound(
        &self,
        original: &rsipstack::sip::Request,
        ctx: &InternalCallContext,
        caller: &SipUser,
        _cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        info!(
            trunk = %ctx.trunk_name,
            tenant_id = ?ctx.tenant_id,
            caller = %caller.username,
            "edge processing outbound call from worker"
        );

        let trunk = self
            .data_context
            .get_trunk(&ctx.trunk_name)
            .ok_or_else(|| {
                RouteError::from((
                    anyhow!("outbound trunk '{}' not found", ctx.trunk_name),
                    Some(rsipstack::sip::StatusCode::NotFound),
                ))
            })?;

        if trunk.disabled.unwrap_or(false) {
            return Err(RouteError::from((
                anyhow!("outbound trunk '{}' is disabled", ctx.trunk_name),
                Some(rsipstack::sip::StatusCode::ServiceUnavailable),
            )));
        }

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
            .unwrap_or_else(|_| format!("edge-{}", std::process::id()));

        let dest_uri = Uri::try_from(trunk.dest.as_str())
            .map_err(|e| RouteError::from((anyhow!("invalid trunk dest '{}': {}", trunk.dest, e), None)))?;

        let credential = match (&trunk.username, &trunk.password) {
            (Some(user), Some(pass)) => Some(Credential {
                username: user.clone(),
                password: pass.clone(),
                realm: dest_uri.host().to_string().into(),
            }),
            _ => None,
        };

        let mut dest_location = Location {
            aor: dest_uri,
            credential,
            ..Default::default()
        };

        if trunk.username.is_some() {
            let pai = rsipstack::sip::Header::Other(
                "P-Asserted-Identity".to_string(),
                format!("<{}>", caller_uri),
            );
            dest_location.headers = Some(vec![pai]);
        }

        let mut dialplan = Dialplan::new(session_id, original.clone(), DialDirection::Outbound)
            .with_caller(caller_uri)
            .with_targets(DialStrategy::Sequential(vec![dest_location]));

        dialplan.media.proxy_mode = MediaProxyMode::None;
        dialplan = dialplan.with_passthrough_failure(true);

        Ok(dialplan)
    }

    async fn resolve_inbound(
        &self,
        original: &rsipstack::sip::Request,
        route_invite: Box<dyn rustpbx::call::RouteInvite>,
        caller: &SipUser,
        cookie: &TransactionCookie,
    ) -> std::result::Result<Dialplan, RouteError> {
        let trunk_ctx = cookie
            .get_extension::<TrunkContext>()
            .ok_or_else(|| RouteError::from((anyhow!("missing trunk context"), None)))?;

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
            .ok_or_else(|| {
                RouteError::from((
                    anyhow!("failed to extract caller URI"),
                    Some(rsipstack::sip::StatusCode::BadRequest),
                ))
            })?;

        // ── Route matching via the DefaultRouteInvite engine ──────────────────
        let preview_option = InviteOption {
            callee: callee_uri.clone(),
            caller: caller_uri.clone(),
            contact: caller_uri.clone(),
            ..Default::default()
        };
        let direction = DialDirection::Inbound;
        let route_result = route_invite
            .preview_route(preview_option, original, &direction, cookie)
            .await
            .map_err(|e| RouteError::from((e, None)))?;

        // ── Map RouteResult → InternalCallContext ─────────────────────────────
        let internal_ctx = self.build_internal_context(
            &route_result,
            &trunk_ctx,
            &caller_uri,
            &callee_uri,
        )?;

        info!(
            call_id = ?original.call_id_header().ok().map(|h| h.value().to_string()),
            trunk = %trunk_ctx.name,
            tenant_id = ?trunk_ctx.tenant_id,
            action = internal_ctx.action.as_str(),
            "edge routed call — dispatching to worker"
        );

        // ── Select Worker ─────────────────────────────────────────────────────
        let worker = self
            .worker_selector
            .select(trunk_ctx.tenant_id)
            .await
            .map_err(|e| {
                warn!(error = %e, "no worker available");
                RouteError::from((
                    e,
                    Some(rsipstack::sip::StatusCode::ServiceUnavailable),
                ))
            })?;

        debug!(worker_id = %worker.worker_id, sip_contact = %worker.sip_contact, "selected worker");

        // ── Build bypass Dialplan targeting the Worker ────────────────────────
        let session_id = original
            .call_id_header()
            .map(|h| h.value().to_string())
            .unwrap_or_else(|_| format!("edge-{}", std::process::id()));

        let worker_uri = Uri::try_from(worker.sip_contact.as_str())
            .map_err(|e| RouteError::from((anyhow!("invalid worker sip_contact: {}", e), None)))?;

        let x_headers = encode_headers(&internal_ctx);

        let worker_location = Location {
            aor: worker_uri,
            headers: Some(x_headers),
            ..Default::default()
        };

        let mut dialplan = Dialplan::new(session_id, original.clone(), DialDirection::Inbound)
            .with_caller(caller_uri)
            .with_targets(DialStrategy::Sequential(vec![worker_location]));

        // Edge is signaling-only — bypass all media handling.
        dialplan.media.proxy_mode = MediaProxyMode::None;
        dialplan.recording = CallRecordingConfig::default();
        // Let SIP failure codes (4xx/5xx from Worker) pass through to the carrier.
        dialplan = dialplan.with_passthrough_failure(true);

        Ok(dialplan)
    }

    /// Translate the routing engine's decision into an `InternalCallContext`
    /// that the Worker can decode.
    fn build_internal_context(
        &self,
        result: &RouteResult,
        trunk_ctx: &TrunkContext,
        caller_uri: &Uri,
        callee_uri: &Uri,
    ) -> std::result::Result<InternalCallContext, RouteError> {
        let (action, targets, app_name, app_params, queue_name) = match result {
            RouteResult::Forward(option, _hints) => {
                let target_str = option.callee.to_string();
                (RouteAction::Forward, vec![target_str], None, None, None)
            }
            RouteResult::Queue { queue, .. } => (
                RouteAction::Queue,
                Vec::new(),
                None,
                None,
                queue.label.clone(),
            ),
            RouteResult::Application {
                app_name, app_params, ..
            } => (
                RouteAction::Application,
                Vec::new(),
                Some(app_name.clone()),
                app_params.clone(),
                None,
            ),
            RouteResult::NotHandled(_, _) => {
                return Err(RouteError::from((
                    anyhow!("no route matched for callee {}", callee_uri),
                    Some(rsipstack::sip::StatusCode::NotFound),
                )));
            }
            RouteResult::Abort(code, reason) => {
                return Err(RouteError::from((
                    anyhow!(reason.clone().unwrap_or_else(|| "route rejected".into())),
                    Some(code.clone()),
                )));
            }
        };

        Ok(InternalCallContext {
            edge_id: self.edge_id.clone(),
            tenant_id: trunk_ctx.tenant_id,
            trunk_name: trunk_ctx.name.clone(),
            trunk_id: trunk_ctx.id,
            direction: InternalDirection::Inbound,
            action,
            original_from: caller_uri.to_string(),
            original_to: callee_uri.to_string(),
            targets,
            dial_strategy: Default::default(),
            app_name,
            app_params,
            queue_name,
            record: false,
            max_duration_secs: None,
        })
    }
}

/// Extract the callee URI: prefer Request-URI's user part, fall back to To header.
/// Mirrors `rustpbx::proxy::call::resolve_callee_uri` (private in the main crate).
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
