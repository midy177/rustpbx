//! Internal call context propagated from Edge to Worker via custom SIP headers.
//!
//! The Edge does ACL + Auth + Route matching, encodes the routing decision into
//! `InternalCallContext`, and injects it as `X-*` headers on the internal INVITE
//! sent to the Worker. The Worker decodes the headers and constructs the
//! appropriate `DialplanFlow` without re-running routing.
//!
//! Header name constants and pure data types live here (no rsipstack dependency).
//! Each binary crate (edge/worker) has a thin adapter that converts between
//! `InternalCallContext` and `rsipstack::sip::Header`.

use serde::{Deserialize, Serialize};

/// X-* header name constants. Keep in sync between edge and worker adapters.
pub const H_EDGE_ID: &str = "X-Edge-Id";
pub const H_TENANT_ID: &str = "X-Tenant-Id";
pub const H_TRUNK_NAME: &str = "X-Trunk-Name";
pub const H_TRUNK_ID: &str = "X-Trunk-Id";
pub const H_DIRECTION: &str = "X-Direction";
pub const H_ROUTE_ACTION: &str = "X-Route-Action";
pub const H_ORIGINAL_FROM: &str = "X-Original-From";
pub const H_ORIGINAL_TO: &str = "X-Original-To";
pub const H_TARGETS: &str = "X-Targets";
pub const H_DIAL_STRATEGY: &str = "X-Dial-Strategy";
pub const H_APP_NAME: &str = "X-App-Name";
pub const H_APP_PARAMS: &str = "X-App-Params";
pub const H_QUEUE_NAME: &str = "X-Queue-Name";
pub const H_RECORD: &str = "X-Record";
pub const H_MAX_DURATION: &str = "X-Max-Duration";

/// What the Edge decided to do with this call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteAction {
    Forward,
    Queue,
    Application,
}

impl Default for RouteAction {
    fn default() -> Self {
        RouteAction::Forward
    }
}

impl RouteAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteAction::Forward => "forward",
            RouteAction::Queue => "queue",
            RouteAction::Application => "application",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "forward" => Some(RouteAction::Forward),
            "queue" => Some(RouteAction::Queue),
            "application" => Some(RouteAction::Application),
            _ => None,
        }
    }
}

/// Dial strategy for Forward action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DialStrategyKind {
    Sequential,
    Parallel,
}

impl Default for DialStrategyKind {
    fn default() -> Self {
        DialStrategyKind::Sequential
    }
}

impl DialStrategyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DialStrategyKind::Sequential => "sequential",
            DialStrategyKind::Parallel => "parallel",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "parallel" => DialStrategyKind::Parallel,
            _ => DialStrategyKind::Sequential,
        }
    }
}

/// Call direction as determined by the Edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InternalDirection {
    Inbound,
    Outbound,
    Internal,
}

impl Default for InternalDirection {
    fn default() -> Self {
        InternalDirection::Inbound
    }
}

impl InternalDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            InternalDirection::Inbound => "inbound",
            InternalDirection::Outbound => "outbound",
            InternalDirection::Internal => "internal",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "inbound" => Some(InternalDirection::Inbound),
            "outbound" => Some(InternalDirection::Outbound),
            "internal" => Some(InternalDirection::Internal),
            _ => None,
        }
    }
}

/// Routing decision payload propagated Edge → Worker.
///
/// All URI strings are full SIP URIs (e.g. `sip:1001@pbx.example.com:5060`).
/// `targets` is comma-separated on the wire when encoded via `H_TARGETS`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InternalCallContext {
    /// Edge instance identifier.
    pub edge_id: String,
    /// Tenant for CDR attribution and isolation.
    pub tenant_id: Option<i64>,
    /// Source trunk name (inbound) or selected trunk name (outbound).
    pub trunk_name: String,
    /// Optional trunk DB id.
    pub trunk_id: Option<i64>,
    /// Call direction.
    pub direction: InternalDirection,
    /// What the Worker should do.
    pub action: RouteAction,
    /// Original caller URI before any rewrite.
    pub original_from: String,
    /// Original callee URI before any rewrite.
    pub original_to: String,
    /// Target URIs for Forward action.
    pub targets: Vec<String>,
    /// Sequential or Parallel dialing.
    pub dial_strategy: DialStrategyKind,
    /// Application name for Application action.
    pub app_name: Option<String>,
    /// Optional JSON-encoded application parameters.
    pub app_params: Option<serde_json::Value>,
    /// Queue name for Queue action.
    pub queue_name: Option<String>,
    /// Whether the Worker should record this call.
    pub record: bool,
    /// Optional maximum call duration in seconds.
    pub max_duration_secs: Option<u64>,
}

/// Flat key-value view used by the wire encoder/decoder.
/// Each tuple is `(header_name, value_string)`.
impl InternalCallContext {
    /// Serialize to a flat list of `(header_name, value)` pairs.
    /// Empty/None fields are omitted to keep the INVITE compact.
    pub fn to_header_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = Vec::with_capacity(12);
        pairs.push((H_EDGE_ID, self.edge_id.clone()));
        if let Some(tid) = self.tenant_id {
            pairs.push((H_TENANT_ID, tid.to_string()));
        }
        pairs.push((H_TRUNK_NAME, self.trunk_name.clone()));
        if let Some(tid) = self.trunk_id {
            pairs.push((H_TRUNK_ID, tid.to_string()));
        }
        pairs.push((H_DIRECTION, self.direction.as_str().to_string()));
        pairs.push((H_ROUTE_ACTION, self.action.as_str().to_string()));
        pairs.push((H_ORIGINAL_FROM, self.original_from.clone()));
        pairs.push((H_ORIGINAL_TO, self.original_to.clone()));
        if !self.targets.is_empty() {
            pairs.push((H_TARGETS, self.targets.join(",")));
        }
        if self.dial_strategy != DialStrategyKind::default() {
            pairs.push((H_DIAL_STRATEGY, self.dial_strategy.as_str().to_string()));
        }
        if let Some(ref app) = self.app_name {
            pairs.push((H_APP_NAME, app.clone()));
        }
        if let Some(ref params) = self.app_params {
            pairs.push((H_APP_PARAMS, params.to_string()));
        }
        if let Some(ref q) = self.queue_name {
            pairs.push((H_QUEUE_NAME, q.clone()));
        }
        if self.record {
            pairs.push((H_RECORD, "true".to_string()));
        }
        if let Some(d) = self.max_duration_secs {
            pairs.push((H_MAX_DURATION, d.to_string()));
        }
        pairs
    }

    /// Decode from an iterator of `(name, value)` pairs (case-insensitive name match).
    /// Returns `None` if the required `X-Route-Action` header is absent
    /// (i.e. this is not an internal call from Edge).
    pub fn from_header_pairs<'a, I>(pairs: I) -> Option<Self>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        use std::collections::HashMap;
        let mut map: HashMap<String, String> = HashMap::new();
        for (name, value) in pairs {
            map.insert(name.to_ascii_lowercase(), value.to_string());
        }
        let get = |name: &str| -> Option<String> { map.get(&name.to_ascii_lowercase()).cloned() };

        let action_str = get(H_ROUTE_ACTION)?;
        let action = RouteAction::from_str(&action_str)?;

        let edge_id = get(H_EDGE_ID)?;
        let trunk_name = get(H_TRUNK_NAME)?;
        let direction = get(H_DIRECTION)
            .as_deref()
            .and_then(InternalDirection::from_str)
            .unwrap_or(InternalDirection::Inbound);

        let original_from = get(H_ORIGINAL_FROM).unwrap_or_default();
        let original_to = get(H_ORIGINAL_TO).unwrap_or_default();

        let targets = get(H_TARGETS)
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        let dial_strategy = get(H_DIAL_STRATEGY)
            .as_deref()
            .map(DialStrategyKind::from_str)
            .unwrap_or_default();

        let tenant_id = get(H_TENANT_ID).and_then(|s| s.parse().ok());
        let trunk_id = get(H_TRUNK_ID).and_then(|s| s.parse().ok());

        let app_name = get(H_APP_NAME);
        let app_params = get(H_APP_PARAMS).and_then(|s| serde_json::from_str(&s).ok());
        let queue_name = get(H_QUEUE_NAME);
        let record = get(H_RECORD)
            .map(|s| s.eq_ignore_ascii_case("true") || s == "1")
            .unwrap_or(false);
        let max_duration_secs = get(H_MAX_DURATION).and_then(|s| s.parse().ok());

        Some(InternalCallContext {
            edge_id,
            tenant_id,
            trunk_name,
            trunk_id,
            direction,
            action,
            original_from,
            original_to,
            targets,
            dial_strategy,
            app_name,
            app_params,
            queue_name,
            record,
            max_duration_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_forward() {
        let ctx = InternalCallContext {
            edge_id: "edge-1".into(),
            tenant_id: Some(42),
            trunk_name: "carrier-a".into(),
            trunk_id: Some(7),
            direction: InternalDirection::Inbound,
            action: RouteAction::Forward,
            original_from: "sip:+861390000@carrier".into(),
            original_to: "sip:1001@pbx".into(),
            targets: vec!["sip:1001@10.0.0.5:5060".into(), "sip:1002@10.0.0.6:5060".into()],
            dial_strategy: DialStrategyKind::Parallel,
            ..Default::default()
        };
        let pairs: Vec<(String, String)> = ctx
            .to_header_pairs()
            .into_iter()
            .map(|(n, v)| (n.to_string(), v))
            .collect();
        let ref_pairs: Vec<(&str, &str)> = pairs.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        let decoded = InternalCallContext::from_header_pairs(ref_pairs).expect("decode");
        assert_eq!(decoded.edge_id, "edge-1");
        assert_eq!(decoded.tenant_id, Some(42));
        assert_eq!(decoded.action, RouteAction::Forward);
        assert_eq!(decoded.targets.len(), 2);
        assert_eq!(decoded.dial_strategy, DialStrategyKind::Parallel);
    }

    #[test]
    fn roundtrip_application() {
        let ctx = InternalCallContext {
            edge_id: "edge-1".into(),
            tenant_id: Some(42),
            trunk_name: "carrier-a".into(),
            direction: InternalDirection::Inbound,
            action: RouteAction::Application,
            original_from: "sip:+861390000@carrier".into(),
            original_to: "sip:400800@carrier".into(),
            app_name: Some("ivr-welcome".into()),
            app_params: Some(serde_json::json!({"lang": "zh"})),
            record: true,
            max_duration_secs: Some(3600),
            ..Default::default()
        };
        let pairs: Vec<(String, String)> = ctx
            .to_header_pairs()
            .into_iter()
            .map(|(n, v)| (n.to_string(), v))
            .collect();
        let ref_pairs: Vec<(&str, &str)> = pairs.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        let decoded = InternalCallContext::from_header_pairs(ref_pairs).expect("decode");
        assert_eq!(decoded.action, RouteAction::Application);
        assert_eq!(decoded.app_name.as_deref(), Some("ivr-welcome"));
        assert_eq!(decoded.record, true);
        assert_eq!(decoded.max_duration_secs, Some(3600));
    }

    #[test]
    fn decode_without_action_returns_none() {
        let pairs = vec![("X-Edge-Id", "edge-1"), ("X-Tenant-Id", "1")];
        assert!(InternalCallContext::from_header_pairs(pairs).is_none());
    }

    #[test]
    fn action_roundtrip() {
        for a in [RouteAction::Forward, RouteAction::Queue, RouteAction::Application] {
            assert_eq!(RouteAction::from_str(a.as_str()), Some(a));
        }
        assert!(RouteAction::from_str("unknown").is_none());
    }
}
