use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::{boolean, json_null, string_len, string_len_null};

/// Create the `rustpbx_ivrs` table — control-plane-owned (the monolith loads
/// IVR definitions from TOML files, not a DB table). `spec` holds the
/// `IvrDefinition` JSON; the worker materializes it to a `{name}.generated.toml`
/// file in its `generated_ivr_dir` for the shared CallModule to read.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_ivrs";

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
                    .col(ColumnDef::new(Alias::new("tenant_id")).big_integer().null())
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
                    .to_owned(),
            )
            .await?;
        // Workers materialize IVRs to files by name (global ivr_dir), so names
        // must be globally unique — index helps the create-time uniqueness check.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .table(Alias::new(TABLE))
                    .name("ux_ivrs_name")
                    .col(Alias::new("name"))
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .table(Alias::new(TABLE))
                    .name("idx_ivrs_tenant_id")
                    .col(Alias::new("tenant_id"))
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
