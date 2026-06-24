use sea_orm_migration::prelude::*;

/// Adds tenant_id column to rustpbx_queues (multi-tenant isolation).
/// Run by the Control Plane migrator. Handles both orderings: whether the
/// table was created by `create_queues` above or by the monolith's queue addon.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        const TABLE: &str = "rustpbx_queues";

        if !manager.has_table(TABLE).await? {
            tracing::warn!(
                table = TABLE,
                "table missing — skipping tenant_id column (create_queues should run first)"
            );
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
                        .name("idx_queues_tenant_id")
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
