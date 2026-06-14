use sea_orm_migration::prelude::*;

/// Adds tenant_id column to rustpbx_routing.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column("rustpbx_routing", "tenant_id")
            .await?
        {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("rustpbx_routing"))
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
                        .table(Alias::new("rustpbx_routing"))
                        .name("idx_routing_tenant_id")
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
