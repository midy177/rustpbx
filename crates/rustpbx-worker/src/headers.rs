//! Thin adapter between `InternalCallContext` and `rsipstack::sip::Header`.
//!
//! The Worker decodes X-* headers from the internal INVITE received from Edge.
//! Encoding is unused on this side but provided for symmetry and testing.

use rustpbx_core::internal::InternalCallContext;

/// Encode an `InternalCallContext` as a vector of SIP headers.
pub fn encode_headers(ctx: &InternalCallContext) -> Vec<rsipstack::sip::Header> {
    ctx.to_header_pairs()
        .into_iter()
        .map(|(name, value)| rsipstack::sip::Header::Other(name.to_string(), value))
        .collect()
}

/// Decode an `InternalCallContext` from a SIP message's headers.
/// Returns `None` if the required `X-Route-Action` header is absent
/// (i.e. this is not an internal call from Edge).
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
    use rustpbx_core::internal::{InternalDirection, RouteAction};

    #[test]
    fn decode_internal_invite() {
        let original = InternalCallContext {
            edge_id: "edge-1".into(),
            tenant_id: Some(42),
            trunk_name: "carrier-a".into(),
            direction: InternalDirection::Inbound,
            action: RouteAction::Forward,
            original_from: "sip:a@b".into(),
            original_to: "sip:c@d".into(),
            targets: vec!["sip:1001@10.0.0.5:5060".into()],
            ..Default::default()
        };
        let headers = encode_headers(&original);
        let sip_headers = rsipstack::sip::Headers::from(headers);
        let decoded = decode_headers(&sip_headers).expect("must decode");
        assert_eq!(decoded.trunk_name, "carrier-a");
        assert_eq!(decoded.action, RouteAction::Forward);
    }
}
