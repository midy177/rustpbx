use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{boolean, string_len_null};

/// Add tenant domain columns (`custom_domain`, `custom_domain_enabled`) to
/// `rustpbx_tenants` for DBs whose tenant table predates them. Idempotent via
/// `has_column` guards — fresh DBs already get the columns from the tenant
/// `create_table`, so this is a no-op there.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_tenants";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_column(TABLE, "custom_domain").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(TABLE))
                        .add_column(string_len_null(Alias::new("custom_domain"), 255))
                        .to_owned(),
                )
                .await?;
        }
        if !manager.has_column(TABLE, "custom_domain_enabled").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(TABLE))
                        .add_column(
                            boolean(Alias::new("custom_domain_enabled")).default(false),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
