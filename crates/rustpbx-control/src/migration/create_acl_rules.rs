use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::{boolean, integer, string_len};

/// Create the `rustpbx_acl_rules` table — IP access-control rules served to
/// Edge/Worker nodes via `GetAclRules`.
///
/// A rule is `<action> <target>` where action is `allow`/`deny` and target is a
/// CIDR or the literal `all` (matching the main binary's `acl_rules` config
/// format, e.g. `"allow 10.0.0.0/8"`, `"deny all"`). `priority` orders
/// evaluation (lower first). Control-owned, self-contained, idempotent.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_acl_rules";

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
                    .col(ColumnDef::new(Alias::new("tenant_id")).big_integer().null())
                    .col(string_len(Alias::new("action"), 8))
                    .col(string_len(Alias::new("target"), 64))
                    .col(integer(Alias::new("priority")).default(100))
                    .col(boolean(Alias::new("is_active")).default(true))
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .timestamp_with_time_zone()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        if !manager.has_index(TABLE, "idx_acl_rules_tenant_id").await? {
            manager
                .create_index(
                    Index::create()
                        .if_not_exists()
                        .name("idx_acl_rules_tenant_id")
                        .table(Alias::new(TABLE))
                        .col(Alias::new("tenant_id"))
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
