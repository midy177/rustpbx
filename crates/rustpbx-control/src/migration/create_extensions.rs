use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::{boolean, integer_null, string_len, string_len_null, text_null};

/// Create the `rustpbx_extensions` base table.
///
/// Mirrors the canonical schema in the main `rustpbx` binary
/// (`src/models/extension.rs`), with a `tenant_id` column added for multi-tenant
/// scoping. Unlike the monolith (which makes `extension` globally unique), the
/// Control Plane scopes uniqueness to `(tenant_id, extension)` so two tenants
/// can each own e.g. extension `1001`.
///
/// NOTE: in a DB shared with the monolith where main created this table first
/// with a *global* unique index, that index still applies — cross-tenant
/// duplicate extensions are only possible in a control-owned DB. Idempotent.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_extensions";

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
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(string_len(Alias::new("extension"), 32))
                    .col(ColumnDef::new(Alias::new("tenant_id")).big_integer().null())
                    .col(string_len_null(Alias::new("display_name"), 160))
                    .col(string_len_null(Alias::new("email"), 160))
                    .col(string_len_null(Alias::new("status"), 32))
                    .col(boolean(Alias::new("login_disabled")).default(false))
                    .col(boolean(Alias::new("voicemail_disabled")).default(false))
                    .col(boolean(Alias::new("allow_guest_calls")).default(false))
                    .col(string_len_null(Alias::new("sip_password"), 160))
                    .col(string_len_null(Alias::new("call_forwarding_mode"), 32))
                    .col(string_len_null(Alias::new("call_forwarding_destination"), 160))
                    .col(integer_null(Alias::new("call_forwarding_timeout")))
                    .col(
                        ColumnDef::new(Alias::new("registered_at"))
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(text_null(Alias::new("notes")))
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
            .await?;

        if !manager
            .has_index(TABLE, "idx_extensions_tenant_extension")
            .await?
        {
            manager
                .create_index(
                    Index::create()
                        .if_not_exists()
                        .unique()
                        .name("idx_extensions_tenant_extension")
                        .table(Alias::new(TABLE))
                        .col(Alias::new("tenant_id"))
                        .col(Alias::new("extension"))
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
