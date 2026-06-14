use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum S3Vendor {
    #[default]
    AWS,
    GCP,
    Azure,
    Aliyun,
    Tencent,
    Minio,
    DigitalOcean,
}

/// Pure data types for policy configuration.
/// Methods that depend on crate-internal types live in the main rustpbx crate.

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct PolicySpec {
    #[serde(default)]
    pub called_prefix: Option<String>,
    #[serde(default)]
    pub trunk_country: Option<String>,
    #[serde(default)]
    pub allowed_destination_countries: Vec<String>,
    #[serde(default)]
    pub time_window: Option<TimeWindow>,
    #[serde(default)]
    pub deny_regions: Vec<String>,
    #[serde(default)]
    pub allow_landline: Option<bool>,
    #[serde(default)]
    pub frequency_limit: Option<FrequencyLimit>,
    #[serde(default)]
    pub daily_limit: Option<DailyLimit>,
    #[serde(default)]
    pub concurrency: Option<ConcurrencyLimit>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TimeWindow {
    pub start: String,
    pub end: String,
    pub timezone: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FrequencyLimit {
    pub count: u32,
    pub window_hours: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DailyLimit {
    pub count: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConcurrencyLimit {
    pub max_total: u32,
    #[serde(default)]
    pub max_per_account: HashMap<String, u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecordingDirection {
    Inbound,
    Outbound,
    Internal,
}

impl RecordingDirection {
    pub fn matches(&self, direction: &crate::routing::DialDirection) -> bool {
        use crate::routing::DialDirection;
        matches!(
            (self, direction),
            (RecordingDirection::Inbound, DialDirection::Inbound)
                | (RecordingDirection::Outbound, DialDirection::Outbound)
                | (RecordingDirection::Internal, DialDirection::Internal)
        )
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecordingType {
    #[default]
    Local,
    Http,
    S3,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct RecordingPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(
        default,
        rename = "type",
        skip_serializing_if = "is_default_recording_type"
    )]
    pub recording_type: RecordingType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directions: Vec<RecordingDirection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caller_allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caller_deny: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callee_allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callee_deny: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_start: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samplerate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ptime: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<S3Vendor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

fn is_default_recording_type(recording_type: &RecordingType) -> bool {
    *recording_type == RecordingType::Local
}
