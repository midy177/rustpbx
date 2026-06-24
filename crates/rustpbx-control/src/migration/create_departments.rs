use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::{json_null, string_len, string_len_null};

/// Create `rustpbx_departments` — the monolith's ExtensionUserBackend JOINs it
/// (via rustpbx_extension_departments) when fetching an extension for REGISTER
/// auth, so the control plane must provision it even though the console doesn't
/// manage departments yet. Mirrors `src/models/department.rs`.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_departments";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .big_integer()
                            .primary_key()
                            .auto_increment(),
                    )
                    .col(string_len(Alias::new("name"), 160))
                    .col(string_len_null(Alias::new("display_label"), 160))
                    .col(string_len_null(Alias::new("slug"), 80))
                    .col(string_len_null(Alias::new("description"), 255))
                    .col(string_len_null(Alias::new("color"), 32))
                    .col(string_len_null(Alias::new("manager_contact"), 160))
                    .col(json_null(Alias::new("metadata")))
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .timestamp_with_time_zone()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Alias::new("updated_at"))
                            .timestamp_with_time_zone()
                            .default(Expr::current_timestamp()),
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
