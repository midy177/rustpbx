pub mod add_tenant_id_to_call_records;
pub mod add_tenant_id_to_routing;
pub mod add_tenant_id_to_trunks;

use sea_orm_migration::{MigrationTrait, MigratorTrait};

pub struct ControlMigrator;

#[async_trait::async_trait]
impl MigratorTrait for ControlMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // New tables (must come before FK references)
            Box::new(crate::models::tenant::Migration),
            Box::new(crate::models::did_number::Migration),
            // Add tenant_id to existing tables shared with main rustpbx
            Box::new(add_tenant_id_to_trunks::Migration),
            Box::new(add_tenant_id_to_routing::Migration),
            Box::new(add_tenant_id_to_call_records::Migration),
        ]
    }
}
