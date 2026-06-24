use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::Expr;
use sea_orm_migration::schema::{boolean, integer_null, json_null, string_len, string_len_null};

/// Create the `rustpbx_sip_trunks` base table.
///
/// The canonical schema lives in the main `rustpbx` binary (`src/models/`),
/// which the Control Plane must not modify. This is a faithful, self-contained
/// copy so the Control Plane can run against a fresh DB (e.g. a control-only
/// SQLite) without the monolith having provisioned it first. Idempotent
/// (`IF NOT EXISTS`); `tenant_id` is added later by `add_tenant_id_to_trunks`.
#[derive(DeriveMigrationName)]
pub struct Migration;

const TABLE: &str = "rustpbx_sip_trunks";

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
                    .col(string_len(Alias::new("name"), 120))
                    .col(string_len_null(Alias::new("display_name"), 160))
                    .col(string_len_null(Alias::new("carrier"), 160))
                    .col(string_len(Alias::new("status"), 32).default("active"))
                    .col(string_len(Alias::new("direction"), 32))
                    .col(string_len_null(Alias::new("sip_server"), 160))
                    .col(string_len(Alias::new("sip_transport"), 16))
                    .col(string_len_null(Alias::new("outbound_proxy"), 160))
                    .col(string_len_null(Alias::new("auth_username"), 160))
                    .col(string_len_null(Alias::new("auth_password"), 160))
                    .col(integer_null(Alias::new("max_cps")))
                    .col(integer_null(Alias::new("max_concurrent")))
                    .col(integer_null(Alias::new("max_call_duration")))
                    .col(json_null(Alias::new("allowed_ips")))
                    .col(json_null(Alias::new("did_numbers")))
                    .col(json_null(Alias::new("tags")))
                    .col(string_len_null(Alias::new("incoming_from_user_prefix"), 160))
                    .col(string_len_null(Alias::new("incoming_to_user_prefix"), 160))
                    .col(boolean(Alias::new("is_active")).default(true))
                    .col(boolean(Alias::new("register_enabled")).default(false))
                    .col(integer_null(Alias::new("register_expires")))
                    .col(json_null(Alias::new("register_extra_headers")))
                    .col(boolean(Alias::new("rewrite_hostport")).default(true))
                    .col(json_null(Alias::new("metadata")))
                    // Columns the monolith's sip_trunk entity expects (reload_trunks
                    // export queries them); kept nullable — the control CRUD doesn't
                    // edit them, they default NULL.
                    .col(string_len_null(Alias::new("description"), 255))
                    .col(string_len_null(Alias::new("default_route_label"), 160))
                    .col(ColumnDef::new(Alias::new("utilisation_percent")).double().null())
                    .col(ColumnDef::new(Alias::new("warning_threshold_percent")).double().null())
                    .col(json_null(Alias::new("billing_snapshot")))
                    .col(json_null(Alias::new("analytics")))
                    .col(
                        ColumnDef::new(Alias::new("last_health_check_at"))
                            .timestamp_with_time_zone()
                            .null(),
                    )
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
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new(TABLE)).to_owned())
            .await
    }
}
