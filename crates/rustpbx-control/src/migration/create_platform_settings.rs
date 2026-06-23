use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::string_len_null;

/// Create `rustpbx_platform_settings` — a tiny key/value store for
/// superadmin-configurable platform settings (currently the wildcard
/// `base_domain` used to mint each tenant's default `{id}.{base_domain}`
/// domain). Control-owned, self-contained, idempotent.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_platform_settings";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Alias::new("key"))
                            .string_len(120)
                            .primary_key(),
                    )
                    .col(string_len_null(Alias::new("value"), 1024))
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
