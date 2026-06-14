use crate::policy::{PolicySpec, RecordingPolicy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConfigOrigin {
    #[default]
    Embedded,
    File(String),
}

impl ConfigOrigin {
    pub fn embedded() -> Self {
        Self::Embedded
    }

    pub fn from_file(path: impl Into<String>) -> Self {
        Self::File(path.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialDirection {
    Outbound,
    Inbound,
    Internal,
}

impl std::fmt::Display for DialDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DialDirection::Outbound => write!(f, "outbound"),
            DialDirection::Inbound => write!(f, "inbound"),
            DialDirection::Internal => write!(f, "internal"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TrunkDirection {
    Inbound,
    Outbound,
    Bidirectional,
}

impl TrunkDirection {
    pub fn allows(&self, direction: &DialDirection) -> bool {
        match self {
            TrunkDirection::Inbound => matches!(direction, DialDirection::Inbound),
            TrunkDirection::Outbound => matches!(direction, DialDirection::Outbound),
            TrunkDirection::Bidirectional => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceTrunk {
    pub name: String,
    pub id: Option<i64>,
    pub direction: Option<TrunkDirection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CacPolicy {
    Lossy,
    Reject,
    Overflow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MediaMode {
    None,
    Bypass,
    Auto,
    ForceTranscode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VideoPolicy {
    PassThrough,
    Strip,
    Transcode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HeaderRule {
    pub action: HeaderAction,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_caller_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_callee_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HeaderAction {
    Add,
    Remove,
    Set,
    Rename,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CallIdMode {
    Transparent,
    Rewrite,
}

fn default_rewrite_hostport() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TrunkConfig {
    pub dest: String,
    pub backup_dest: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codec: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_calls: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<u32>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<TrunkDirection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbound_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<RecordingPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incoming_from_user_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incoming_to_user_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_expires: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_extra_headers: Option<HashMap<String, String>>,
    #[serde(default = "default_rewrite_hostport")]
    pub rewrite_hostport: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id_mode: Option<CallIdMode>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_per_ip: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_interval_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_probe_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check_fallback_trunk: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cac_policy: Option<CacPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow_threshold: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_rules: Option<Vec<HeaderRule>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_mode: Option<MediaMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_policy: Option<VideoPolicy>,

    #[serde(skip)]
    pub origin: ConfigOrigin,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub did_numbers: Vec<String>,
}

impl Default for TrunkConfig {
    fn default() -> Self {
        Self {
            dest: String::new(),
            backup_dest: None,
            username: None,
            password: None,
            codec: Vec::new(),
            disabled: None,
            max_calls: None,
            max_cps: None,
            weight: None,
            transport: None,
            id: None,
            direction: None,
            inbound_hosts: Vec::new(),
            recording: None,
            incoming_from_user_prefix: None,
            incoming_to_user_prefix: None,
            country: None,
            policy: None,
            register_enabled: None,
            register_expires: None,
            register_extra_headers: None,
            rewrite_hostport: true,
            call_id_mode: None,
            health_check_enabled: None,
            health_check_per_ip: None,
            health_check_interval_secs: None,
            health_check_probe_count: None,
            health_check_fallback_trunk: None,
            cac_policy: None,
            overflow_threshold: None,
            header_rules: None,
            media_mode: None,
            video_policy: None,
            did_numbers: Vec::new(),
            origin: ConfigOrigin::embedded(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum RouteDirection {
    #[default]
    Any,
    Inbound,
    Outbound,
}

impl RouteDirection {
    pub fn matches(&self, direction: &DialDirection) -> bool {
        match self {
            RouteDirection::Any => true,
            RouteDirection::Inbound => matches!(direction, DialDirection::Inbound),
            RouteDirection::Outbound => matches!(direction, DialDirection::Outbound),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct MatchConditions {
    #[serde(rename = "from.user")]
    pub from_user: Option<String>,
    #[serde(rename = "from.host")]
    pub from_host: Option<String>,
    #[serde(rename = "to.user")]
    pub to_user: Option<String>,
    #[serde(rename = "to.host")]
    pub to_host: Option<String>,
    #[serde(rename = "to.port")]
    pub to_port: Option<String>,
    #[serde(rename = "request_uri.user")]
    pub request_uri_user: Option<String>,
    #[serde(rename = "request_uri.host")]
    pub request_uri_host: Option<String>,
    #[serde(rename = "request_uri.port")]
    pub request_uri_port: Option<String>,
    #[serde(flatten)]
    pub headers: HashMap<String, String>,

    pub from: Option<String>,
    pub to: Option<String>,
    pub caller: Option<String>,
    pub callee: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct RewriteRules {
    #[serde(rename = "from.user")]
    pub from_user: Option<String>,
    #[serde(rename = "from.host")]
    pub from_host: Option<String>,
    #[serde(rename = "to.user")]
    pub to_user: Option<String>,
    #[serde(rename = "to.host")]
    pub to_host: Option<String>,
    #[serde(rename = "to.port")]
    pub to_port: Option<String>,
    #[serde(rename = "request_uri.user")]
    pub request_uri_user: Option<String>,
    #[serde(rename = "request_uri.host")]
    pub request_uri_host: Option<String>,
    #[serde(rename = "request_uri.port")]
    pub request_uri_port: Option<String>,
    #[serde(flatten)]
    pub headers: HashMap<String, String>,
}

fn default_select() -> String {
    "rr".to_string()
}

fn default_auto_answer() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RouteAction {
    #[serde(default)]
    pub action: Option<String>,

    #[serde(default)]
    pub dest: Option<DestConfig>,

    #[serde(default = "default_select")]
    pub select: String,

    #[serde(default)]
    pub hash_key: Option<String>,

    #[serde(default)]
    pub reject: Option<RejectConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_params: Option<serde_json::Value>,

    #[serde(default = "default_auto_answer")]
    pub auto_answer: bool,
}

impl Default for RouteAction {
    fn default() -> Self {
        RouteAction {
            action: None,
            dest: None,
            select: default_select(),
            hash_key: None,
            reject: None,
            queue: None,
            app: None,
            app_params: None,
            auto_answer: default_auto_answer(),
        }
    }
}

impl RouteAction {
    pub fn get_action_type(&self) -> ActionType {
        match &self.action {
            Some(action) => match action.as_str() {
                "reject" => ActionType::Reject,
                "busy" => ActionType::Busy,
                "queue" => ActionType::Queue,
                "application" => ActionType::Application,
                _ => ActionType::Forward,
            },
            None => {
                if self.app.is_some() {
                    ActionType::Application
                } else if self.queue.is_some() {
                    ActionType::Queue
                } else if self.reject.is_some() {
                    ActionType::Reject
                } else {
                    ActionType::Forward
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActionType {
    Forward,
    Reject,
    Busy,
    Queue,
    Application,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct RejectConfig {
    pub code: u16,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum DestConfig {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RouteRule {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub direction: RouteDirection,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_trunks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_trunk_ids: Vec<i64>,

    #[serde(rename = "match")]
    pub match_conditions: MatchConditions,

    #[serde(default)]
    pub rewrite: Option<RewriteRules>,

    #[serde(flatten)]
    pub action: RouteAction,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codecs: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_ice_servers: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicySpec>,
    #[serde(skip)]
    pub origin: ConfigOrigin,
}

impl Default for RouteRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: None,
            priority: 0,
            direction: RouteDirection::Any,
            source_trunks: Vec::new(),
            source_trunk_ids: Vec::new(),
            match_conditions: MatchConditions::default(),
            rewrite: None,
            action: RouteAction::default(),
            codecs: Vec::new(),
            disable_ice_servers: None,
            disabled: None,
            policy: None,
            origin: ConfigOrigin::embedded(),
        }
    }
}
