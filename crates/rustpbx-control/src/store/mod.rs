pub mod crud;
pub mod db_queries;

pub use db_queries::CdrListOpts;

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

// ── Admin views (HTTP API) ──────────────────────────────────────────────────
// Secret-safe projections of the shared rustpbx_* tables for the admin console.
// Passwords are never serialized — only `has_auth` indicates credentials exist.

#[derive(Debug, Clone, serde::Serialize)]
pub struct TrunkView {
    pub id: i64,
    pub name: String,
    /// Resolved destination (sip_server, else outbound_proxy).
    pub dest: Option<String>,
    pub transport: String,
    pub direction: String,
    pub has_auth: bool,
    pub register_enabled: bool,
    pub is_active: bool,
    pub did_numbers: Vec<String>,
    pub allowed_ips: Vec<String>,
    pub max_concurrent: Option<i32>,
    pub tenant_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteView {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub priority: i32,
    pub direction: String,
    pub source_pattern: Option<String>,
    pub destination_pattern: Option<String>,
    pub target_trunks: Vec<String>,
    pub is_active: bool,
    pub tenant_id: Option<i64>,
}

/// CDR row for the admin console (secret-safe projection).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CdrView {
    pub id: i64,
    pub call_id: String,
    pub tenant_id: Option<i64>,
    pub direction: String,
    pub status: String,
    pub from_number: Option<String>,
    pub to_number: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_secs: i32,
    pub recording_url: Option<String>,
}

/// CDR record to persist.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
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
