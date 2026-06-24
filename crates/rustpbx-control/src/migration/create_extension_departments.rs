use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;

/// Create `rustpbx_extension_departments` — the extension↔department join table
/// the monolith's ExtensionUserBackend LEFT JOINs when fetching an extension
/// (src/proxy/user_extension.rs). Without it, REGISTER auth queries error out
/// and the worker reports "User not found". Mirrors
/// `src/models/extension_department.rs`. Composite PK (extension_id, department_id).
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_extension_departments";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .if_not_exists()
                    .col(ColumnDef::new(Alias::new("extension_id")).big_integer().not_null())
                    .col(ColumnDef::new(Alias::new("department_id")).big_integer().not_null())
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .timestamp_with_time_zone()
                            .default(Expr::current_timestamp()),
                    )
                    .primary_key(
                        Index::create()
                            .col(Alias::new("extension_id"))
                            .col(Alias::new("department_id")),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new(TABLE)).to_owned())
            .await
    }
}
