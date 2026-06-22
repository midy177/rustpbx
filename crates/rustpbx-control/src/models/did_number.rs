use sea_orm::entity::prelude::*;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;
use sea_orm_migration::sea_query::ForeignKeyAction;
use serde::{Deserialize, Serialize};

// ── DID status ────────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[sea_orm(rs_type = "String", db_type = "Text")]
#[derive(Default)]
pub enum DidStatus {
    #[default]
    #[sea_orm(string_value = "available")]
    Available,
    #[sea_orm(string_value = "assigned")]
    Assigned,
    #[sea_orm(string_value = "reserved")]
    Reserved,
    #[sea_orm(string_value = "porting")]
    Porting,
}

// ── Entity ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "rustpbx_did_numbers")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique)]
    pub number: String,
    pub tenant_id: Option<i64>,
    pub trunk_id: Option<i64>,
    pub status: DidStatus,
    pub country: Option<String>,
    pub city: Option<String>,
    pub monthly_cost: Option<i32>,
    pub metadata: Option<Json>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
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
                    .col(string(Column::Number).unique_key())
                    .col(big_integer_null(Column::TenantId))
                    .col(big_integer_null(Column::TrunkId))
                    .col(string(Column::Status).default("available"))
                    .col(string_null(Column::Country))
                    .col(string_null(Column::City))
                    .col(integer_null(Column::MonthlyCost))
                    .col(json_null(Column::Metadata))
                    .col(timestamp(Column::CreatedAt).default(Expr::current_timestamp()))
                    .col(timestamp(Column::UpdatedAt).default(Expr::current_timestamp()))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_did_tenant")
                            .from(Entity, Column::TenantId)
                            .to(super::tenant::Entity, super::tenant::Column::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .table(Entity)
                    .name("idx_did_tenant_id")
                    .col(Column::TenantId)
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
