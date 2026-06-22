pub mod add_tenant_id_to_call_records;
pub mod add_tenant_id_to_routing;
pub mod add_tenant_id_to_trunks;
pub mod create_call_records;
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
            // Base tables (self-contained copy of the main binary's schema, so
            // control can run against a fresh DB without the monolith). Must
            // come before the add_tenant_id migrations that ALTER them.
            Box::new(create_sip_trunks::Migration),
            Box::new(create_routing::Migration),
            Box::new(create_call_records::Migration),
            // Add tenant_id to the base tables
            Box::new(add_tenant_id_to_trunks::Migration),
            Box::new(add_tenant_id_to_routing::Migration),
            Box::new(add_tenant_id_to_call_records::Migration),
        ]
    }
}
