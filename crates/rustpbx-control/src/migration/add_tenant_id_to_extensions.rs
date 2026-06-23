use sea_orm_migration::prelude::*;

/// Adds `tenant_id` to `rustpbx_extensions` for DBs where the monolith created
/// the table first (without it). On a control-owned DB the column already comes
/// from `create_extensions`, so the `has_column` guard makes this a no-op.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_extensions";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_table(TABLE).await? {
            return Ok(());
        }
        if !manager.has_column(TABLE, "tenant_id").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(TABLE))
                        .add_column(ColumnDef::new(Alias::new("tenant_id")).big_integer().null())
                        .to_owned(),
                )
                .await?;

            manager
                .create_index(
                    Index::create()
                        .if_not_exists()
                        .table(Alias::new(TABLE))
                        .name("idx_extensions_tenant_id")
                        .col(Alias::new("tenant_id"))
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
