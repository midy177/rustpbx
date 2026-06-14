//! Thin adapter between `InternalCallContext` and `rsipstack::sip::Header`.
//!
//! The Edge encodes routing decisions into X-* headers on the internal INVITE
//! sent to the Worker. This module converts `InternalCallContext` (pure data,
//! defined in `rustpbx-core`) to/from `rsipstack::sip::Header::Other`.

use rustpbx_core::internal::InternalCallContext;

/// Encode an `InternalCallContext` as a vector of SIP headers for the outbound INVITE.
pub fn encode_headers(ctx: &InternalCallContext) -> Vec<rsipstack::sip::Header> {
    ctx.to_header_pairs()
        .into_iter()
        .map(|(name, value)| rsipstack::sip::Header::Other(name.to_string(), value))
        .collect()
}

/// Decode an `InternalCallContext` from a SIP message's headers.
/// Returns `None` if the required `X-Route-Action` header is absent.
pub fn decode_headers(headers: &rsipstack::sip::Headers) -> Option<InternalCallContext> {
    let pairs: Vec<(&str, &str)> = headers
        .iter()
        .filter_map(|h| match h {
            rsipstack::sip::Header::Other(n, v) => Some((n.as_str(), v.as_str())),
            _ => None,
        })
        .collect();
    InternalCallContext::from_header_pairs(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustpbx_core::internal::{DialStrategyKind, InternalDirection, RouteAction};

    #[test]
    fn encode_decode_roundtrip() {
        let ctx = InternalCallContext {
            edge_id: "edge-1".into(),
            tenant_id: Some(42),
            trunk_name: "carrier-a".into(),
            direction: InternalDirection::Inbound,
            action: RouteAction::Application,
            original_from: "sip:+861390000@carrier".into(),
            original_to: "sip:400800@carrier".into(),
            app_name: Some("ivr-welcome".into()),
            targets: vec!["sip:1001@10.0.0.5:5060".into()],
            dial_strategy: DialStrategyKind::Parallel,
            ..Default::default()
        };
        let headers = encode_headers(&ctx);
        let sip_headers = rsipstack::sip::Headers::from(headers);
        let decoded = decode_headers(&sip_headers).expect("must decode");
        assert_eq!(decoded.edge_id, "edge-1");
        assert_eq!(decoded.action, RouteAction::Application);
        assert_eq!(decoded.app_name.as_deref(), Some("ivr-welcome"));
        assert_eq!(decoded.dial_strategy, DialStrategyKind::Parallel);
        assert_eq!(decoded.targets.len(), 1);
    }

    #[test]
    fn decode_non_internal_returns_none() {
        let headers = rsipstack::sip::Headers::from(vec![
            rsipstack::sip::Header::Other("User-Agent".into(), "test".into()),
        ]);
        assert!(decode_headers(&headers).is_none());
    }
}
