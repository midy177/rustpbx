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
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::{debug, info, warn};

pub struct EdgeCallRouter {
    pub worker_selector: Arc<WorkerSelector>,
    pub data_context: Arc<ProxyDataContext>,
    #[allow(dead_code)]
    pub routing_state: Arc<RoutingState>,
    pub edge_id: String,
    /// Live count of in-flight proxied calls, reported in the edge heartbeat.
    /// Incremented here on a successful route; decremented by
    /// `EdgeActiveCallHook` when the call's CDR completes.
    pub active_calls: Arc<AtomicU32>,
}

impl EdgeCallRouter {
    pub fn new(
        worker_selector: Arc<WorkerSelector>,
        data_context: Arc<ProxyDataContext>,
        routing_state: Arc<RoutingState>,
        edge_id: String,
        active_calls: Arc<AtomicU32>,
    ) -> Self {
        Self { worker_selector, data_context, routing_state, edge_id, active_calls }
    }

    /// Reserve a slot on the selected worker via `AllocateCall`, returning the
    /// SIP contact the worker wants the INVITE sent to.
    ///
    /// Best-effort: if the worker doesn't advertise an EdgeWorker addr, returns
    /// the worker's pre-known `sip_contact` (no reservation — backward
    /// compatible). A rejection (`accepted=false`) or transport error is
    /// surfaced as an error so the caller can fail the call rather than send an
    /// INVITE the worker won't accept.
    async fn allocate_on_worker(
        &self,
        worker: &crate::worker_selector::WorkerEndpoint,
        call_id: &str,
        tenant_id: Option<i64>,
        caller: &str,
        callee: &str,
    ) -> Result<String> {
        use rustpbx_proto::edge::{
            edge_worker_client::EdgeWorkerClient, AllocateCallRequest,
        };

        if worker.edge_worker_addr.trim().is_empty() {
            // Worker doesn't serve AllocateCall — use the known SIP contact.
            return Ok(worker.sip_contact.clone());
        }

        let endpoint = if worker.edge_worker_addr.starts_with("http") {
            worker.edge_worker_addr.clone()
        } else {
            format!("http://{}", worker.edge_worker_addr)
        };
        let mut client = EdgeWorkerClient::connect(endpoint)
            .await
            .map_err(|e| anyhow!("connect AllocateCall: {e}"))?;
        let resp = client
            .allocate_call(AllocateCallRequest {
                call_id: call_id.to_string(),
                tenant_id: tenant_id.unwrap_or(0),
                trunk_name: String::new(),
                caller: caller.to_string(),
                callee: callee.to_string(),
                direction: "inbound".to_string(),
                custom_headers: Default::default(),
            })
            .await
            .map_err(|e| anyhow!("AllocateCall rpc: {e}"))?
            .into_inner();

        if !resp.accepted {
            return Err(anyhow!(
                "worker rejected allocation: {}",
                resp.reject_reason.unwrap_or_default()
            ));
        }
        // Prefer the worker's returned contact; fall back to the known one.
        if resp.worker_sip_contact.trim().is_empty() {
            Ok(worker.sip_contact.clone())
        } else {
            Ok(resp.worker_sip_contact)
        }
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

        // The dialed number (callee) is carried in the internal context — the
        // INVITE's Request-URI points at the Edge, not the PSTN number. Prefer
        // the encoded targets, then original_to, then fall back to the message.
        let callee_uri = ctx
            .targets
            .first()
            .and_then(|t| Uri::try_from(t.as_str()).ok())
            .or_else(|| Uri::try_from(ctx.original_to.as_str()).ok())
            .or_else(|| resolve_callee_uri(original).ok())
            .ok_or_else(|| {
                RouteError::from((
                    anyhow!("outbound: cannot resolve dialed callee"),
                    Some(rsipstack::sip::StatusCode::BadRequest),
                ))
            })?;
        let caller_uri = Uri::try_from(ctx.original_from.as_str())
            .ok()
            .or_else(|| caller.from.clone())
            .or_else(|| original.from_header().ok().and_then(|h| h.uri().ok()))
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

        // Carrier Request-URI = dialed number's user part @ trunk dest host.
        // Mirrors the monolith's `apply_trunk_config` host-port rewrite so the
        // carrier sees `sip:<number>@<carrier-host>`.
        let carrier_aor = build_carrier_aor(&callee_uri, &dest_uri);

        let mut dest_location = Location {
            aor: carrier_aor,
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

        self.active_calls.fetch_add(1, Ordering::Relaxed);
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

        // Reserve a slot on the worker (AllocateCall) and learn the exact SIP
        // contact to target. Falls back to the known contact when the worker
        // doesn't serve AllocateCall.
        let worker_contact = self
            .allocate_on_worker(
                &worker,
                &session_id,
                trunk_ctx.tenant_id,
                &caller_uri.to_string(),
                &callee_uri.to_string(),
            )
            .await
            .map_err(|e| {
                warn!(worker_id = %worker.worker_id, error = %e, "worker allocation failed");
                RouteError::from((e, Some(rsipstack::sip::StatusCode::ServiceUnavailable)))
            })?;

        let worker_uri = Uri::try_from(worker_contact.as_str())
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

        self.active_calls.fetch_add(1, Ordering::Relaxed);
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

/// Build the carrier-facing Request-URI: keep the dialed number (callee user
/// part) and swap the host/port to the trunk's destination. Mirrors the
/// `rewrite_hostport` behaviour of the monolith's `apply_trunk_config`.
fn build_carrier_aor(callee: &Uri, trunk_dest: &Uri) -> Uri {
    let mut aor = callee.clone();
    aor.host_with_port = trunk_dest.host_with_port.clone();
    aor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carrier_aor_keeps_dialed_number_and_swaps_host() {
        let callee = Uri::try_from("sip:+8613800138000@worker.local:5070").unwrap();
        let trunk_dest = Uri::try_from("sip:carrier.example.com:5060").unwrap();
        let aor = build_carrier_aor(&callee, &trunk_dest);

        // dialed number preserved
        assert_eq!(aor.user().unwrap(), "+8613800138000");
        // host/port routed to the carrier
        assert_eq!(aor.host_with_port.to_string(), "carrier.example.com:5060");
    }
}
