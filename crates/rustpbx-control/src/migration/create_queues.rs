use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::{boolean, json_null, string_len, string_len_null};

/// Create the `rustpbx_queues` base table.
///
/// Canonical schema is owned by the main `rustpbx` queue addon
/// (`src/addons/queue/models.rs`), which the Control Plane must not modify. This
/// is a faithful self-contained copy so the Control Plane can run against a
/// fresh DB without the monolith. Idempotent (`IF NOT EXISTS`); `tenant_id` is
/// added by `add_tenant_id_to_queues`. Name uniqueness is enforced at the
/// application layer (per-tenant) to avoid clashing with the addon's own
/// constraint when the two share a DB.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_queues";

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
                    .col(string_len_null(Alias::new("description"), 255))
                    .col(json_null(Alias::new("metadata")))
                    .col(json_null(Alias::new("spec")))
                    .col(boolean(Alias::new("is_active")).default(true))
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
                    .col(
                        ColumnDef::new(Alias::new("last_exported_at"))
                            .timestamp_with_time_zone()
                            .null(),
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
