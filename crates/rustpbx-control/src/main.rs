mod config;
mod grpc;
mod http_api;
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
    http_api::HttpState,
    migration::ControlMigrator,
    store::Store,
    worker_registry::WorkerRegistry,
};
use anyhow::Result;
use dashmap::DashMap;
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
    // `DatabaseConnection` is cheaply clonable (Arc inside) — keep a handle for
    // the HTTP API while `Store` owns its own clone.
    let store = Arc::new(Store::new(db.clone()));
    let workers = Arc::new(WorkerRegistry::new(Duration::from_secs(30)));

    let svc = ControlPlaneService::new(Arc::clone(&store), Arc::clone(&workers));
    let grpc_svc = ControlPlaneServer::new(svc);

    // ── HTTP admin API + SPA ────────────────────────────────────────────────
    let http_state = HttpState {
        db,
        store: Arc::clone(&store),
        workers: Arc::clone(&workers),
        sessions: Arc::new(DashMap::new()),
        admin_username: cfg.admin_username.clone(),
        admin_password: cfg.admin_password.clone(),
    };
    let http_router = http_api::build_router(http_state, &cfg.web_dir);
    let http_addr: std::net::SocketAddr = cfg.http_addr.parse()?;

    // ── gRPC Server ───────────────────────────────────────────────────────────
    let grpc_addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;

    info!(%grpc_addr, %http_addr, web_dir = %cfg.web_dir, "control plane listening");

    let grpc_server = tonic::transport::Server::builder()
        .add_service(grpc_svc)
        .serve(grpc_addr);

    let http_server = async move {
        let listener = tokio::net::TcpListener::bind(http_addr).await?;
        axum::serve(listener, http_router).await?;
        Ok::<(), anyhow::Error>(())
    };

    // Run both; if either exits/errors, shut down.
    tokio::select! {
        r = grpc_server => { r?; }
        r = http_server => { r?; }
    }

    Ok(())
}
