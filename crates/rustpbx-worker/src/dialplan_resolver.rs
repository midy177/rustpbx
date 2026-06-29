//! Worker dialplan construction helpers.
//!
//! This module keeps the pure "routing decision -> Dialplan" logic separate
//! from `WorkerCallRouter`, which owns runtime concerns such as metrics,
//! reservations, and active-call counters.

use anyhow::{Result, anyhow};
use rsipstack::sip::Uri;
use rsipstack::sip::prelude::HeadersExt;
use rustpbx::call::cookie::{TenantId, TrunkContext};
use rustpbx::call::{DialDirection, DialStrategy, Dialplan, Location, QueuePlan, user::SipUser};
use rustpbx::config::{MediaProxyMode, RtpConfig};
use rustpbx_core::internal::{
    DialStrategyKind, InternalCallContext, InternalDirection, RouteAction,
};
use tracing::warn;

pub fn build_internal_dialplan<F>(
    original: &rsipstack::sip::Request,
    ctx: &InternalCallContext,
    caller: &SipUser,
    rtp_config: &RtpConfig,
    load_queue_plan: F,
) -> Result<Dialplan>
where
    F: FnOnce(&str) -> Result<QueuePlan>,
{
    let callee_uri = resolve_callee_uri(original)?;
    let caller_uri = caller
        .from
        .clone()
        .or_else(|| original.from_header().ok().and_then(|h| h.uri().ok()))
        .unwrap_or_else(|| callee_uri.clone());

    let session_id = original
        .call_id_header()
        .map(|h| h.value().to_string())
        .unwrap_or_else(|_| format!("worker-{}", std::process::id()));

    let dial_direction = match ctx.direction {
        InternalDirection::Inbound => DialDirection::Inbound,
        InternalDirection::Outbound => DialDirection::Outbound,
        InternalDirection::Internal => DialDirection::Internal,
    };

    let mut dialplan =
        Dialplan::new(session_id, original.clone(), dial_direction).with_caller(caller_uri);

    match ctx.action {
        RouteAction::Forward => {
            let locations = build_forward_locations(ctx, &callee_uri);
            if locations.is_empty() {
                return Err(anyhow!("forward action with no targets"));
            }
            let strategy = match ctx.dial_strategy {
                DialStrategyKind::Parallel => DialStrategy::Parallel(locations),
                DialStrategyKind::Sequential => DialStrategy::Sequential(locations),
            };
            dialplan = dialplan.with_targets(strategy);
        }
        RouteAction::Application => {
            let app_name = ctx
                .app_name
                .clone()
                .ok_or_else(|| anyhow!("application action without app_name"))?;
            dialplan = dialplan.with_application(app_name, ctx.app_params.clone(), true);
        }
        RouteAction::Queue => {
            let queue_name = ctx
                .queue_name
                .clone()
                .ok_or_else(|| anyhow!("queue action without queue_name"))?;
            let queue_plan = load_queue_plan(&queue_name)?;
            dialplan = dialplan.with_queue(queue_plan);
        }
    }

    apply_worker_media(&mut dialplan, rtp_config);
    Ok(apply_internal_call_options(dialplan, ctx))
}

pub(crate) fn apply_worker_media(dialplan: &mut Dialplan, rtp_config: &RtpConfig) {
    dialplan.media.proxy_mode = MediaProxyMode::All;
    dialplan.media.external_ip = rtp_config.external_ip.clone();
    dialplan.media.rtp_start_port = rtp_config.start_port;
    dialplan.media.rtp_end_port = rtp_config.end_port;
}

fn apply_internal_call_options(mut dialplan: Dialplan, ctx: &InternalCallContext) -> Dialplan {
    if ctx.record {
        dialplan.recording.enabled = true;
        dialplan.recording.auto_start = true;
    }

    if let Some(secs) = ctx.max_duration_secs {
        dialplan.max_call_duration = Some(std::time::Duration::from_secs(secs));
    }

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
    dialplan
}

pub(crate) fn build_forward_locations(
    ctx: &InternalCallContext,
    fallback_callee: &Uri,
) -> Vec<Location> {
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

/// Extract the callee URI: prefer Request-URI's user part, fall back to To header.
pub(crate) fn resolve_callee_uri(origin: &rsipstack::sip::Request) -> Result<Uri> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_targets(targets: Vec<String>) -> InternalCallContext {
        InternalCallContext {
            action: RouteAction::Forward,
            targets,
            ..Default::default()
        }
    }

    #[test]
    fn forward_locations_use_fallback_when_targets_empty() {
        let fallback = Uri::try_from("sip:1001@example.com").unwrap();
        let locations = build_forward_locations(&ctx_with_targets(Vec::new()), &fallback);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].aor.to_string(), "sip:1001@example.com");
    }

    #[test]
    fn forward_locations_preserve_target_order() {
        let fallback = Uri::try_from("sip:fallback@example.com").unwrap();
        let ctx = ctx_with_targets(vec![
            "sip:1001@example.com".to_string(),
            "sip:1002@example.com".to_string(),
        ]);
        let locations = build_forward_locations(&ctx, &fallback);
        let uris: Vec<String> = locations.into_iter().map(|l| l.aor.to_string()).collect();
        assert_eq!(uris, vec!["sip:1001@example.com", "sip:1002@example.com"]);
    }
}
