use sea_orm_migration::prelude::*;

/// Adds tenant_id column to rustpbx_call_records.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Check the actual table name used by the main crate
        let table = "rustpbx_call_records";

        // The base table is created by the main `rustpbx` binary; if this DB
        // hasn't been provisioned yet (control-only / fresh SQLite), skip the
        // ALTER instead of crashing. Re-running control after main provisions
        // the schema will add the column.
        if !manager.has_table(table).await? {
            tracing::warn!(
                table,
                "base table missing — skipping tenant_id column (run the main rustpbx binary to provision the shared schema)"
            );
            return Ok(());
        }

        if !manager.has_column(table, "tenant_id").await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(table))
                        .add_column(
                            ColumnDef::new(Alias::new("tenant_id"))
                                .big_integer()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;

            manager
                .create_index(
                    Index::create()
                        .table(Alias::new(table))
                        .name("idx_call_records_tenant_id")
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
