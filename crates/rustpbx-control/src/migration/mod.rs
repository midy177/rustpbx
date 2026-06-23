pub mod add_domain_to_tenants;
pub mod add_tenant_id_to_call_records;
pub mod add_tenant_id_to_extensions;
pub mod add_tenant_id_to_routing;
pub mod add_tenant_id_to_trunks;
pub mod create_acl_rules;
pub mod create_call_records;
pub mod create_extensions;
pub mod create_platform_settings;
pub mod create_routing;
pub mod create_sip_trunks;

use sea_orm_migration::{MigrationTrait, MigratorTrait};

pub struct ControlMigrator;

#[async_trait::async_trait]
impl MigratorTrait for ControlMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // Control-owned tables
            Box::new(crate::models::tenant::Migration),
            Box::new(crate::models::did_number::Migration),
            Box::new(crate::models::tenant_user::Migration),
            Box::new(create_platform_settings::Migration),
            // Base tables (self-contained copy of the main binary's schema, so
            // control can run against a fresh DB without the monolith). Must
            // come before the add_tenant_id migrations that ALTER them.
            Box::new(create_sip_trunks::Migration),
            Box::new(create_routing::Migration),
            Box::new(create_call_records::Migration),
            Box::new(create_acl_rules::Migration),
            Box::new(create_extensions::Migration),
            // Add tenant_id / domain columns to the base/own tables
            Box::new(add_tenant_id_to_trunks::Migration),
            Box::new(add_tenant_id_to_routing::Migration),
            Box::new(add_tenant_id_to_call_records::Migration),
            Box::new(add_tenant_id_to_extensions::Migration),
            Box::new(add_domain_to_tenants::Migration),
        ]
    }
}
