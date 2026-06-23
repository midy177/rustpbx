use sea_orm::entity::prelude::*;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;
use serde::{Deserialize, Serialize};

// ── Tenant status ─────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[sea_orm(rs_type = "String", db_type = "Text")]
#[derive(Default)]
pub enum TenantStatus {
    #[default]
    #[sea_orm(string_value = "active")]
    Active,
    #[sea_orm(string_value = "suspended")]
    Suspended,
    #[sea_orm(string_value = "deleted")]
    Deleted,
}

// ── Entity ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "rustpbx_tenants")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    pub status: TenantStatus,
    pub max_concurrent_calls: Option<i32>,
    pub max_trunks: Option<i32>,
    pub max_dids: Option<i32>,
    pub storage_prefix: Option<String>,
    /// Tenant-chosen SIP/PBX domain. When set and `custom_domain_enabled`, it is
    /// the tenant's active domain; the auto-assigned `{id}.{base_domain}` default
    /// is then paused-but-reserved (never handed to another tenant).
    pub custom_domain: Option<String>,
    /// Whether the custom domain is the active one. False → the default
    /// (wildcard-derived) domain is active even if a custom domain is stored.
    pub custom_domain_enabled: bool,
    pub metadata: Option<Json>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
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
                    .col(string(Column::Name).unique_key())
                    .col(string(Column::Status).default("active"))
                    .col(integer_null(Column::MaxConcurrentCalls))
                    .col(integer_null(Column::MaxTrunks))
                    .col(integer_null(Column::MaxDids))
                    .col(string_null(Column::StoragePrefix))
                    .col(string_null(Column::CustomDomain))
                    .col(boolean(Column::CustomDomainEnabled).default(false))
                    .col(json_null(Column::Metadata))
                    .col(timestamp(Column::CreatedAt).default(Expr::current_timestamp()))
                    .col(timestamp(Column::UpdatedAt).default(Expr::current_timestamp()))
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
