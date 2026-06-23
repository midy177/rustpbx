//! Audit-log entity + idempotent migration.
//!
//! Every privileged mutation (create/update/delete on tenants, users, trunks,
//! routes, extensions, ACL, DIDs, domains) writes one row here so there's a
//! durable, queryable trail of who changed what. Tenant-scoped: tenant
//! principals see only their own tenant's entries; the superadmin sees all.

use sea_orm::entity::prelude::*;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;
use serde::{Deserialize, Serialize};

// ── Entity ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "rustpbx_audit_log")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Who performed the action (username from the session).
    pub actor_username: String,
    /// Session role ("superadmin" / tenant role).
    pub actor_role: String,
    /// Tenant the actor belonged to (NULL = superadmin / platform).
    pub actor_tenant_id: Option<i64>,
    /// "create" | "update" | "delete".
    pub action: String,
    /// Resource kind: "tenant" | "trunk" | "route" | "extension" | "acl" |
    /// "did" | "tenant_user" | "domain".
    pub target_type: String,
    /// Primary key of the affected row (NULL when unknown, e.g. delete by id).
    pub target_id: Option<i64>,
    /// Short human summary, e.g. "created trunk 'acme-sip'".
    pub summary: String,
    /// JSON snapshot of the change (post-mutation state). Secrets scrubbed.
    pub payload: Option<Json>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

// ── Migration ─────────────────────────────────────────────────────────────────

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Entity)
                    .if_not_exists()
                    .col(pk_auto(Column::Id))
                    .col(string(Column::ActorUsername))
                    .col(string(Column::ActorRole))
                    .col(big_integer_null(Column::ActorTenantId))
                    .col(string(Column::Action))
                    .col(string(Column::TargetType))
                    .col(big_integer_null(Column::TargetId))
                    .col(string(Column::Summary))
                    .col(json_null(Column::Payload))
                    .col(timestamp(Column::CreatedAt).default(Expr::current_timestamp()))
                    .to_owned(),
            )
            .await?;

        // Tenant-scoped browsing + recency lookups.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .table(Entity)
                    .name("idx_audit_tenant_created")
                    .col(Column::ActorTenantId)
                    .col(Column::CreatedAt)
                    .to_owned(),
            )
            .await?;
        // Find the history of a specific resource.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .table(Entity)
                    .name("idx_audit_target")
                    .col(Column::TargetType)
                    .col(Column::TargetId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Entity).to_owned())
            .await
    }
}
