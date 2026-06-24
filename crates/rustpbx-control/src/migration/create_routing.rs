use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::{boolean, integer, json_null, string_len, string_len_null};

/// Create the `rustpbx_routes` base table.
///
/// Self-contained copy of the main binary's schema (see `create_sip_trunks`
/// for rationale). Idempotent; `tenant_id` is added later by
/// `add_tenant_id_to_routing`.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_routes";

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
                    .col(string_len(Alias::new("direction"), 32))
                    .col(integer(Alias::new("priority")).default(100))
                    .col(boolean(Alias::new("is_active")).default(true))
                    .col(string_len(Alias::new("selection_strategy"), 32))
                    .col(string_len_null(Alias::new("hash_key"), 120))
                    .col(ColumnDef::new(Alias::new("source_trunk_id")).big_integer().null())
                    .col(ColumnDef::new(Alias::new("default_trunk_id")).big_integer().null())
                    .col(string_len_null(Alias::new("source_pattern"), 160))
                    .col(string_len_null(Alias::new("destination_pattern"), 160))
                    .col(json_null(Alias::new("header_filters")))
                    .col(json_null(Alias::new("rewrite_rules")))
                    .col(json_null(Alias::new("target_trunks")))
                    .col(string_len_null(Alias::new("owner"), 120))
                    .col(json_null(Alias::new("notes")))
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
