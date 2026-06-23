//! Tenant IAM accounts — `rustpbx_tenant_users`.
//!
//! AWS-IAM-style sub-accounts that belong to a tenant. Each carries a bcrypt
//! password hash, a role (`admin` / `user`), and (for plain users) a set of
//! granted permission strings. The tenant's first `admin` is provisioned when
//! the superadmin creates the tenant; that admin then manages further users.
//!
//! `username` is unique *within a tenant* (composite `(tenant_id, username)`).
//! Login carries the tenant's domain, which resolves to a `tenant_id`; the
//! username is then matched inside that tenant.

use sea_orm::entity::prelude::*;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "rustpbx_tenant_users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub tenant_id: i64,
    /// Unique within the tenant (see the composite index in the migration).
    pub username: String,
    pub display_name: Option<String>,
    #[serde(skip_serializing)]
    pub password_hash: String,
    /// `admin` or `user` (see `auth::permissions::db_role`).
    pub role: String,
    /// JSON array of granted permission strings (only meaningful for `user`).
    pub permissions: Option<Json>,
    /// `active` or `suspended`.
    pub status: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub last_login_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tenant::Entity",
        from = "Column::TenantId",
        to = "super::tenant::Column::Id"
    )]
    Tenant,
}

impl Related<super::tenant::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tenant.def()
    }
}

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
                    .col(big_integer(Column::TenantId))
                    .col(string(Column::Username))
                    .col(string_null(Column::DisplayName))
                    .col(string(Column::PasswordHash))
                    .col(string(Column::Role).default("user"))
                    .col(json_null(Column::Permissions))
                    .col(string(Column::Status).default("active"))
                    .col(timestamp(Column::CreatedAt).default(Expr::current_timestamp()))
                    .col(timestamp(Column::UpdatedAt).default(Expr::current_timestamp()))
                    .col(timestamp_null(Column::LastLoginAt))
                    .to_owned(),
            )
            .await?;

        // Username is unique within a tenant, not globally.
        if !manager
            .has_index("rustpbx_tenant_users", "idx_tenant_users_tenant_username")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .if_not_exists()
                        .unique()
                        .name("idx_tenant_users_tenant_username")
                        .table(Entity)
                        .col(Column::TenantId)
                        .col(Column::Username)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Entity).to_owned())
            .await
    }
}
