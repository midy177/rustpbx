use sea_orm::entity::prelude::*;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    integer_null, string_len, string_len_null, timestamp_with_time_zone as timestamp,
};
use sea_orm_migration::sea_query::ColumnDef;
use sea_query::Expr;
use serde::{Deserialize, Serialize};

pub const DEFAULT_TENANT_SLUG: &str = "default";
pub const DEFAULT_TENANT_NAME: &str = "Default";

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "rustpbx_tenants")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i64,
    #[sea_orm(unique)]
    pub slug: String,
    pub name: String,
    pub status: String,
    pub domain: Option<String>,
    pub max_concurrent_calls: Option<i32>,
    pub max_trunks: Option<i32>,
    pub storage_prefix: Option<String>,
    #[sea_orm(column_type = "DateTime", default_value = "CURRENT_TIMESTAMP")]
    pub created_at: DateTimeUtc,
    #[sea_orm(column_type = "DateTime", default_value = "CURRENT_TIMESTAMP")]
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

pub async fn ensure_default_tenant<C>(db: &C) -> Result<Model, DbErr>
where
    C: ConnectionTrait,
{
    if let Some(existing) = Entity::find()
        .filter(Column::Slug.eq(DEFAULT_TENANT_SLUG))
        .one(db)
        .await?
    {
        return Ok(existing);
    }

    let now = chrono::Utc::now();
    ActiveModel {
        slug: Set(DEFAULT_TENANT_SLUG.to_string()),
        name: Set(DEFAULT_TENANT_NAME.to_string()),
        status: Set("active".to_string()),
        domain: Set(None),
        max_concurrent_calls: Set(None),
        max_trunks: Set(None),
        storage_prefix: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Entity)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Column::Id)
                            .big_integer()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(string_len(Column::Slug, 120).unique_key())
                    .col(string_len(Column::Name, 255))
                    .col(string_len(Column::Status, 32).default("active"))
                    .col(string_len_null(Column::Domain, 255))
                    .col(integer_null(Column::MaxConcurrentCalls))
                    .col(integer_null(Column::MaxTrunks))
                    .col(string_len_null(Column::StoragePrefix, 255))
                    .col(timestamp(Column::CreatedAt).default(Expr::current_timestamp()))
                    .col(timestamp(Column::UpdatedAt).default(Expr::current_timestamp()))
                    .to_owned(),
            )
            .await?;

        ensure_default_tenant(manager.get_connection()).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Entity).to_owned())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::migration::Migrator;
    use sea_orm::Database;
    use sea_orm_migration::MigratorTrait;

    #[tokio::test]
    async fn default_tenant_is_seeded_once() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite memory");

        Migrator::up(&db, None).await.expect("run migrations");
        let first = ensure_default_tenant(&db).await.expect("default tenant");
        let second = ensure_default_tenant(&db).await.expect("default tenant");
        let tenants = Entity::find().all(&db).await.expect("query tenants");

        assert_eq!(first.id, second.id);
        assert_eq!(tenants.len(), 1);
        assert_eq!(tenants[0].slug, DEFAULT_TENANT_SLUG);
        assert_eq!(tenants[0].status, "active");
    }
}
