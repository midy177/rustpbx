use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_tenant_id(
            manager,
            "rustpbx_users",
            super::user::Entity,
            super::user::Column::TenantId,
            "idx_rustpbx_users_tenant_id",
        )
        .await?;
        add_tenant_id(
            manager,
            "rustpbx_extensions",
            super::extension::Entity,
            super::extension::Column::TenantId,
            "idx_rustpbx_extensions_tenant_id",
        )
        .await?;
        add_tenant_id(
            manager,
            "rustpbx_sip_trunks",
            super::sip_trunk::Entity,
            super::sip_trunk::Column::TenantId,
            "idx_rustpbx_sip_trunks_tenant_id",
        )
        .await?;
        add_tenant_id(
            manager,
            "rustpbx_routes",
            super::routing::Entity,
            super::routing::Column::TenantId,
            "idx_rustpbx_routes_tenant_id",
        )
        .await?;
        add_tenant_id(
            manager,
            "rustpbx_call_records",
            super::call_record::Entity,
            super::call_record::Column::TenantId,
            "idx_rustpbx_call_records_tenant_id",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

async fn add_tenant_id<T, C>(
    manager: &SchemaManager<'_>,
    table_name: &'static str,
    table: T,
    column: C,
    index_name: &'static str,
) -> Result<(), DbErr>
where
    T: IntoTableRef + Copy,
    C: IntoIden + Copy,
{
    if !manager.has_column(table_name, "tenant_id").await? {
        manager
            .alter_table(
                Table::alter()
                    .table(table)
                    .add_column(ColumnDef::new(column).big_integer().null())
                    .to_owned(),
            )
            .await?;
    }

    manager
        .create_index(
            Index::create()
                .if_not_exists()
                .name(index_name)
                .table(table)
                .col(column)
                .to_owned(),
        )
        .await?;

    Ok(())
}
