//! Audit-log service over `rustpbx_audit_log`.
//!
//! Records privileged mutations and lists them (tenant-scoped for tenant
//! principals, all entries for the superadmin). Writes are best-effort from the
//! HTTP handlers — a failed audit insert must never roll back a successful
//! mutation, so callers use `.ok()` / log on error rather than propagating.

use crate::models::audit_log::{ActiveModel, Column, Entity, Model};
use anyhow::Result;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::Serialize;
use serde_json::Value;

/// One auditable event, as constructed by the HTTP handlers.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub actor_username: String,
    pub actor_role: String,
    pub actor_tenant_id: Option<i64>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<i64>,
    pub summary: String,
    /// Optional JSON snapshot (post-mutation state, secrets scrubbed).
    pub payload: Option<Value>,
}

impl AuditEntry {
    /// Convenience constructor so handler call-sites stay terse.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        actor_username: impl Into<String>,
        actor_role: impl Into<String>,
        actor_tenant_id: Option<i64>,
        action: impl Into<String>,
        target_type: impl Into<String>,
        target_id: Option<i64>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            actor_username: actor_username.into(),
            actor_role: actor_role.into(),
            actor_tenant_id,
            action: action.into(),
            target_type: target_type.into(),
            target_id,
            summary: summary.into(),
            payload: None,
        }
    }

    /// Build an entry without actor identity — the HTTP layer fills the actor
    /// from the session (`HttpState::audit`). This is the form handlers use.
    pub fn action(
        action: impl Into<String>,
        target_type: impl Into<String>,
        target_id: Option<i64>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            actor_username: String::new(),
            actor_role: String::new(),
            actor_tenant_id: None,
            action: action.into(),
            target_type: target_type.into(),
            target_id,
            summary: summary.into(),
            payload: None,
        }
    }

    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// A serialized audit row returned to the frontend.
#[derive(Debug, Serialize)]
pub struct AuditResponse {
    pub id: i64,
    pub actor_username: String,
    pub actor_role: String,
    pub actor_tenant_id: Option<i64>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<i64>,
    pub summary: String,
    pub payload: Option<Value>,
    pub created_at: String,
}

impl From<Model> for AuditResponse {
    fn from(m: Model) -> Self {
        Self {
            id: m.id,
            actor_username: m.actor_username,
            actor_role: m.actor_role,
            actor_tenant_id: m.actor_tenant_id,
            action: m.action,
            target_type: m.target_type,
            target_id: m.target_id,
            summary: m.summary,
            payload: m.payload,
            created_at: m.created_at.to_rfc3339(),
        }
    }
}

/// Optional filters for listing audit entries.
#[derive(Debug, Clone)]
pub struct AuditFilter {
    /// Restrict to a tenant (None = all — superadmin scope).
    pub tenant_id: Option<i64>,
    pub action: Option<String>,
    pub target_type: Option<String>,
    pub limit: u64,
}

impl Default for AuditFilter {
    fn default() -> Self {
        Self { tenant_id: None, action: None, target_type: None, limit: 100 }
    }
}

pub struct AuditService<'a> {
    db: &'a DatabaseConnection,
}

impl<'a> AuditService<'a> {
    pub fn new(db: &'a DatabaseConnection) -> Self {
        Self { db }
    }

    /// Persist one audit entry. Returns the stored model.
    pub async fn record(&self, entry: AuditEntry) -> Result<Model> {
        let now = Utc::now();
        let am = ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            actor_username: Set(entry.actor_username),
            actor_role: Set(entry.actor_role),
            actor_tenant_id: Set(entry.actor_tenant_id),
            action: Set(entry.action),
            target_type: Set(entry.target_type),
            target_id: Set(entry.target_id),
            summary: Set(entry.summary),
            payload: Set(entry.payload),
            created_at: Set(now),
        };
        Ok(am.insert(self.db).await?)
    }

    /// List audit entries matching `filter`, newest first.
    pub async fn list(&self, filter: &AuditFilter) -> Result<Vec<AuditResponse>> {
        let limit = filter.limit.clamp(1, 500) as u64;
        let mut q = Entity::find().order_by_desc(Column::CreatedAt);
        if let Some(tid) = filter.tenant_id {
            q = q.filter(Column::ActorTenantId.eq(tid));
        }
        if let Some(a) = filter.action.as_deref() {
            if !a.is_empty() {
                q = q.filter(Column::Action.eq(a));
            }
        }
        if let Some(t) = filter.target_type.as_deref() {
            if !t.is_empty() {
                q = q.filter(Column::TargetType.eq(t));
            }
        }
        Ok(q.limit(limit).all(self.db).await?.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh_db() -> DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        // Bring up the full control schema (the audit table + its indices).
        use sea_orm_migration::MigratorTrait;
        crate::migration::ControlMigrator::up(&db, None).await.unwrap();
        db
    }

    fn entry(tenant: Option<i64>, action: &str, target: &str) -> AuditEntry {
        AuditEntry::new(
            "alice",
            "tenant_admin",
            tenant,
            action,
            target,
            Some(42),
            format!("{action} {target}"),
        )
        .with_payload(serde_json::json!({ "name": "acme" }))
    }

    #[tokio::test]
    async fn record_and_list_scopes_by_tenant() {
        let db = fresh_db().await;
        let svc = AuditService::new(&db);
        svc.record(entry(Some(1), "create", "trunk")).await.unwrap();
        svc.record(entry(Some(1), "update", "trunk")).await.unwrap();
        svc.record(entry(Some(2), "delete", "route")).await.unwrap();
        svc.record(entry(None, "create", "tenant")).await.unwrap();

        // Tenant 1 sees only its own two entries.
        let t1 = svc.list(&AuditFilter { tenant_id: Some(1), limit: 100, ..Default::default() }).await.unwrap();
        assert_eq!(t1.len(), 2);
        assert!(t1.iter().all(|e| e.actor_tenant_id == Some(1)));

        // Superadmin (no scope) sees all four, newest first.
        let all = svc.list(&AuditFilter { limit: 100, ..Default::default() }).await.unwrap();
        assert_eq!(all.len(), 4);
        // created_at desc ordering: the last-inserted ("create tenant") is first.
        assert_eq!(all[0].target_type, "tenant");

        // Action + target_type filters compose.
        let f = AuditFilter {
            tenant_id: Some(1),
            action: Some("update".into()),
            target_type: Some("trunk".into()),
            limit: 100,
            ..Default::default()
        };
        assert_eq!(svc.list(&f).await.unwrap().len(), 1);

        // Payload round-trips.
        let with_payload = svc.list(&AuditFilter { tenant_id: Some(1), limit: 1, ..Default::default() }).await.unwrap();
        assert_eq!(with_payload[0].payload.as_ref().unwrap()["name"], "acme");
    }

    #[tokio::test]
    async fn list_limit_is_clamped() {
        let db = fresh_db().await;
        let svc = AuditService::new(&db);
        for _ in 0..5 {
            svc.record(entry(Some(9), "create", "trunk")).await.unwrap();
        }
        let rows = svc.list(&AuditFilter { tenant_id: Some(9), limit: 2, ..Default::default() }).await.unwrap();
        assert_eq!(rows.len(), 2, "limit caps the page");
    }
}
