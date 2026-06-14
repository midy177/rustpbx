mod config;
mod grpc;
mod migration;
mod models;
mod store;
mod tenant_service;
mod worker_registry;

use crate::{
    grpc::{
        control_plane::ControlPlaneService,
        proto::control::control_plane_server::ControlPlaneServer,
    },
    migration::ControlMigrator,
    store::Store,
    worker_registry::WorkerRegistry,
};
use anyhow::Result;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use std::sync::Arc;
use tokio::time::Duration;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rustpbx-control.toml".to_string());

    let cfg = if std::path::Path::new(&config_path).exists() {
        config::ControlConfig::load(&config_path)?
    } else {
        config::ControlConfig::default()
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.log.as_str().into()),
        )
        .init();

    info!(grpc_addr = %cfg.grpc_addr, http_addr = %cfg.http_addr, "rustpbx-control starting");

    // ── Database + Migrations ─────────────────────────────────────────────────
    let db = Database::connect(&cfg.database_url).await?;
    info!(database_url = %cfg.database_url, "database connected");

    ControlMigrator::up(&db, None).await?;
    info!("control plane migrations applied");

    // ── Services ──────────────────────────────────────────────────────────────
    let store = Arc::new(Store::new(db));
    let workers = Arc::new(WorkerRegistry::new(Duration::from_secs(30)));

    let svc = ControlPlaneService::new(Arc::clone(&store), Arc::clone(&workers));
    let grpc_svc = ControlPlaneServer::new(svc);

    // ── gRPC Server ───────────────────────────────────────────────────────────
    let grpc_addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;
    info!(%grpc_addr, "gRPC server listening");

    tonic::transport::Server::builder()
        .add_service(grpc_svc)
        .serve(grpc_addr)
        .await?;

    Ok(())
}
