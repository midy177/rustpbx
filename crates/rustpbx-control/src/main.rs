mod auth;
mod config;
mod did_service;
mod grpc;
mod http_api;
mod migration;
mod models;
mod raft;
mod settings;
mod store;
mod tenant_service;
mod tenant_user_service;

use crate::{
    grpc::{
        control_plane::ControlPlaneService,
        proto::control::control_plane_server::ControlPlaneServer,
    },
    http_api::HttpState,
    migration::ControlMigrator,
    raft::registry::RaftRegistry,
    store::Store,
};
use anyhow::Result;
use dashmap::DashMap;
use sea_orm::Database;
use sea_orm_migration::{MigratorTrait, SchemaManager};
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
    // Demote SeaORM/sqlx statement logging to DEBUG so per-query lines don't
    // spam the default `info` level (set `log=debug` to see them again).
    let mut db_opt = sea_orm::ConnectOptions::new(&cfg.database_url);
    db_opt.sqlx_logging_level(tracing::log::LevelFilter::Debug);
    let db = Database::connect(db_opt).await?;
    info!(database_url = %cfg.database_url, "database connected");

    // Initialize schema directly, bypassing SeaORM's migration version tracking.
    //
    // The shared DB is normally created by the main `rustpbx` binary (22 base
    // migrations); the Control Plane only adds its tenant tables/columns on top.
    // SeaORM's `Migrator::up` requires *every* applied migration to be in the
    // current migrator's list, so running `ControlMigrator::up` against a DB
    // already migrated by main fails with "applied migration file missing".
    //
    // Each Control migration is idempotent (`if_not_exists` / `has_column`
    // guards), so we just run their `up()` DDL through a SchemaManager — no
    // `seaql_migrations` bookkeeping, no cross-migrator conflict.
    let manager = SchemaManager::new(&db);
    for m in ControlMigrator::migrations() {
        m.up(&manager).await?;
    }
    info!("control plane schema initialized");

    // Seed the wildcard base_domain from config on first start (superadmin edits
    // via the API thereafter take precedence — see PlatformSettings::seed).
    settings::PlatformSettings::new(&db)
        .seed_base_domain(&cfg.base_domain)
        .await?;

    // ── Services ──────────────────────────────────────────────────────────────
    // `DatabaseConnection` is cheaply clonable (Arc inside) — keep a handle for
    // the HTTP API while `Store` owns its own clone.
    let store = Arc::new(Store::new(db.clone()));

    // Raft backing the worker registry. Replicated cluster state (high-churn,
    // ephemeral) lives here; persistent config/CDR stays in the DB.
    //
    // No `raft.addr` → single-node mode (backward compatible). With an addr,
    // start cluster mode: the seed node (id 1) bootstraps a single-voter
    // cluster, others start uninitialized and are added at runtime via the
    // admin API (`add_learner` + `change_membership`).
    let hb_timeout = Duration::from_secs(cfg.heartbeat_timeout_secs);
    let workers = if cfg.raft.is_cluster_mode() {
        let node_id = cfg.raft.node_id;
        let advertise = cfg.raft.effective_advertise_addr().to_string();
        let bootstrap = node_id == 1;
        // The business gRPC addr is advertised alongside the Raft addr so a
        // follower can forward writes to the leader's ControlPlane service.
        let client_tls = cfg.tls.is_enabled().then(|| cfg.tls.client_tls()).transpose()?;
        RaftRegistry::start_cluster(
            node_id,
            &advertise,
            &cfg.grpc_addr,
            bootstrap,
            hb_timeout,
            client_tls,
        )
        .await?
    } else {
        RaftRegistry::start(cfg.raft.node_id, hb_timeout).await?
    };

    // In cluster mode, run the dedicated Raft gRPC server so peers can reach us.
    if cfg.raft.is_cluster_mode() {
        let raft_addr: std::net::SocketAddr = cfg.raft.addr.parse()?;
        let raft_server = raft::server::RaftServer::new(workers.raft().clone());
        let tls = cfg.tls.is_enabled().then(|| cfg.tls.server_tls()).transpose()?;
        info!(%raft_addr, node_id = cfg.raft.node_id, tls = tls.is_some(), "raft transport listening");
        tokio::spawn(async move {
            let svc = grpc::proto::raft::raft_service_server::RaftServiceServer::new(raft_server);
            let mut builder = tonic::transport::Server::builder();
            if let Some(tls) = tls {
                builder = match builder.tls_config(tls) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::error!(error = %e, "raft TLS config invalid");
                        return;
                    }
                };
            }
            if let Err(e) = builder.add_service(svc).serve(raft_addr).await {
                tracing::error!(error = %e, "raft transport server exited");
            }
        });
    }

    // Background reaper: remove workers whose heartbeat has been stale for
    // >2× the timeout (default 30s unhealthy → 60s reaped), so dead media
    // nodes don't accumulate in the registry / admin API. Reaping is a Raft
    // write, so it's a no-op on followers' behalf — only the leader commits it.
    let reap_registry = workers.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.tick().await; // discard the immediate first tick
        loop {
            tick.tick().await;
            if let Err(e) = reap_registry.reap_stale().await {
                tracing::warn!(error = %e, "worker reap failed");
            }
            if let Err(e) = reap_registry.reap_stale_edges().await {
                tracing::warn!(error = %e, "edge reap failed");
            }
        }
    });

    let svc = ControlPlaneService::new(Arc::clone(&store), workers.clone());
    let grpc_svc = ControlPlaneServer::new(svc);

    // ── HTTP admin API + SPA ────────────────────────────────────────────────
    let http_state = HttpState {
        db,
        store: Arc::clone(&store),
        workers: workers.clone(),
        sessions: Arc::new(DashMap::new()),
        login_gate: Arc::new(DashMap::new()),
        admin_username: cfg.admin_username.clone(),
        admin_password: cfg.admin_password.clone(),
    };
    let http_router = http_api::build_router(http_state, &cfg.web_dir);
    let http_addr: std::net::SocketAddr = cfg.http_addr.parse()?;

    // ── gRPC Server ───────────────────────────────────────────────────────────
    let grpc_addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;

    info!(%grpc_addr, %http_addr, web_dir = %cfg.web_dir, tls = cfg.tls.is_enabled(), "control plane listening");

    let mut grpc_builder = tonic::transport::Server::builder();
    if cfg.tls.is_enabled() {
        grpc_builder = grpc_builder.tls_config(cfg.tls.server_tls()?)?;
    }
    let grpc_server = grpc_builder.add_service(grpc_svc).serve(grpc_addr);

    let http_server = async move {
        let listener = tokio::net::TcpListener::bind(http_addr).await?;
        // ConnectInfo (peer addr) is needed for the login rate-limiter.
        axum::serve(
            listener,
            http_router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    };

    // Run both; if either exits/errors, shut down.
    tokio::select! {
        r = grpc_server => { r?; }
        r = http_server => { r?; }
    }

    Ok(())
}
