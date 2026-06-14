pub mod db_queries;

use sea_orm::DatabaseConnection;

/// Thin DB access layer for the Control Plane.
///
/// Queries trunk and route records from the shared database.
/// Uses the same sea-orm entities as the main rustpbx crate — the Control
/// Plane runs against the same schema.
pub struct Store {
    pub db: DatabaseConnection,
}

impl Store {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

// ── Trunk queries ─────────────────────────────────────────────────────────────

/// Minimal trunk view returned to Edge instances.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrunkRow {
    pub id: i64,
    pub name: String,
    pub sip_server: Option<String>,
    pub outbound_proxy: Option<String>,
    pub sip_transport: String,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    pub direction: String,
    pub register_enabled: bool,
    pub register_expires: Option<i32>,
    pub rewrite_hostport: bool,
    pub allowed_ips: Option<serde_json::Value>,
    pub did_numbers: Option<serde_json::Value>,
    pub incoming_from_user_prefix: Option<String>,
    pub incoming_to_user_prefix: Option<String>,
    pub tenant_id: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

/// Minimal route view.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteRow {
    pub id: i64,
    pub name: String,
    pub priority: i32,
    pub direction: Option<String>,
    pub source_pattern: Option<String>,
    pub destination_pattern: Option<String>,
    pub target_trunks: Option<serde_json::Value>,
    pub source_trunk_ids: Option<serde_json::Value>,
    pub rewrite_rules: Option<serde_json::Value>,
    pub header_filters: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

/// CDR record to persist.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CdrInsert {
    pub call_id: String,
    pub tenant_id: Option<i64>,
    pub caller: String,
    pub callee: String,
    pub direction: String,
    pub status: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub answer_time: Option<chrono::DateTime<chrono::Utc>>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_secs: Option<i32>,
    pub trunk_name: Option<String>,
    pub worker_id: Option<String>,
    pub edge_id: Option<String>,
    pub recording_path: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub hangup_cause: Option<i32>,
}
