use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::{
    boolean, integer, integer_null, json_null, string_len, string_len_null, text_null,
};

/// Create the `rustpbx_call_records` base table.
///
/// Self-contained copy of the main binary's CDR schema (see `create_sip_trunks`
/// for rationale), so the Control Plane can persist Worker-reported CDRs against
/// a fresh DB. Column names match the canonical schema in `src/models/` exactly
/// (`started_at` / `ended_at` / `sip_trunk_id`, …) so a monolith reading the
/// same DB sees consistent rows.
///
/// Foreign keys to department/extension/sip_trunk/routing are intentionally
/// omitted: those base tables may not exist in a control-only DB, and control
/// only INSERTs CDRs (no referential navigation needed). Idempotent.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_call_records";

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
                    .col(string_len(Alias::new("call_id"), 120))
                    .col(string_len_null(Alias::new("display_id"), 120))
                    .col(string_len(Alias::new("direction"), 16))
                    .col(string_len(Alias::new("status"), 32))
                    .col(
                        ColumnDef::new(Alias::new("started_at"))
                            .timestamp_with_time_zone()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Alias::new("ended_at"))
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(integer(Alias::new("duration_secs")).not_null().default(0))
                    .col(string_len_null(Alias::new("from_number"), 64))
                    .col(string_len_null(Alias::new("to_number"), 64))
                    .col(string_len_null(Alias::new("caller_name"), 160))
                    .col(string_len_null(Alias::new("agent_name"), 160))
                    .col(string_len_null(Alias::new("queue"), 120))
                    .col(ColumnDef::new(Alias::new("department_id")).big_integer().null())
                    .col(ColumnDef::new(Alias::new("extension_id")).big_integer().null())
                    .col(ColumnDef::new(Alias::new("sip_trunk_id")).big_integer().null())
                    .col(ColumnDef::new(Alias::new("route_id")).big_integer().null())
                    .col(string_len_null(Alias::new("sip_gateway"), 160))
                    .col(string_len_null(Alias::new("rewrite_original_from"), 128))
                    .col(string_len_null(Alias::new("rewrite_original_to"), 128))
                    .col(text_null(Alias::new("caller_uri")))
                    .col(text_null(Alias::new("callee_uri")))
                    .col(string_len_null(Alias::new("recording_url"), 255))
                    .col(integer_null(Alias::new("recording_duration_secs")))
                    .col(boolean(Alias::new("has_transcript")).default(false))
                    .col(string_len(Alias::new("transcript_status"), 32).default("pending"))
                    .col(string_len_null(Alias::new("transcript_language"), 16))
                    .col(json_null(Alias::new("tags")))
                    .col(json_null(Alias::new("leg_timeline")))
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
                    .col(
                        ColumnDef::new(Alias::new("archived_at"))
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        if !manager
            .has_index(TABLE, "idx_rustpbx_call_records_call_id")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .if_not_exists()
                        .name("idx_rustpbx_call_records_call_id")
                        .table(Alias::new(TABLE))
                        .col(Alias::new("call_id"))
                        .unique()
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new(TABLE)).to_owned())
            .await
    }
}
