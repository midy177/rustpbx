mod call_router;
mod config;
mod config_source;
mod config_watcher;
mod grpc_client;
mod headers;
mod internal_peer;
mod proto;
mod worker_selector;

use crate::{
    call_router::EdgeCallRouter,
    config::EdgeConfig,
    config_source::{ConfigSource, GrpcConfigSource},
    config_watcher::run_config_watcher,
    grpc_client::GrpcControlClient,
    internal_peer::InternalPeerModule,
    internal_peer::init_trusted_workers,
    worker_selector::WorkerSelector,
};
use anyhow::Result;
use ipnetwork::IpNetwork;
use rustpbx::{
    call::RoutingState,
    config::ProxyConfig,
    proxy::{
        acl::AclModule,
        auth::AuthModule,
        call::CallModule,
        data::ProxyDataContext,
        server::SipServerBuilder,
    },
};
use std::str::FromStr;
use std::sync::Arc;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rustpbx-edge.toml".to_string());

    let cfg = if std::path::Path::new(&config_path).exists() {
        EdgeConfig::load(&config_path)?
    } else {
        EdgeConfig::default()
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.log.as_str().into()),
        )
        .init();

    info!(
        edge_id = %cfg.edge_id,
        sip = format!("{}:{}", cfg.sip_addr, cfg.udp_port),
        control_plane = %cfg.control_plane_addr,
        trusted_workers = cfg.trusted_workers.len(),
        "rustpbx-edge starting"
    );

    // ── Initialise trusted Worker list (OnceLock — must be before SipServer build) ─
    let trusted_networks: Vec<IpNetwork> = cfg
        .trusted_workers
        .iter()
        .filter_map(|s| match IpNetwork::from_str(s) {
            Ok(net) => Some(net),
            Err(e) => {
                warn!(entry = %s, error = %e, "invalid trusted_workers entry, skipping");
                None
            }
        })
        .collect();
    init_trusted_workers(trusted_networks);

    // ── Connect to Control Plane ──────────────────────────────────────────────
    let grpc_client = GrpcControlClient::connect(
        &cfg.control_plane_addr,
        cfg.edge_id.clone(),
    ).await?;
    let grpc_client = Arc::new(tokio::sync::RwLock::new(grpc_client));

    // ── GrpcConfigSource ──────────────────────────────────────────────────────
    let config_source = GrpcConfigSource::new(
        GrpcControlClient::connect(&cfg.control_plane_addr, cfg.edge_id.clone()).await?,
        None,
    );

    // ── Worker selector ───────────────────────────────────────────────────────
    let worker_selector = Arc::new(WorkerSelector::new(Arc::clone(&grpc_client)));

    // ── CancellationToken ─────────────────────────────────────────────────────
    let cancel = CancellationToken::new();

    // ── ProxyDataContext (shared by SipServer + EdgeCallRouter) ───────────────
    let proxy_config = Arc::new(build_proxy_config(&cfg));
    let data_context = Arc::new(
        ProxyDataContext::new(proxy_config.clone(), None)
            .await
            .map_err(|e| anyhow::anyhow!("failed to init data context: {e}"))?,
    );

    // ── Initial config pull from Control Plane → inject into data_context ─────
    inject_grpc_config(&config_source, &data_context).await;

    // ── RoutingState (round-robin / hash state for trunk selection) ───────────
    let routing_state = Arc::new(RoutingState::new());

    // ── EdgeCallRouter ────────────────────────────────────────────────────────
    let edge_router = Box::new(EdgeCallRouter::new(
        Arc::clone(&worker_selector),
        Arc::clone(&data_context),
        Arc::clone(&routing_state),
        cfg.edge_id.clone(),
    ));

    // ── Config watcher (background, re-pulls + re-injects on change) ──────────
    let config_source = Arc::new(config_source);
    tokio::spawn(run_config_watcher(
        Arc::clone(&grpc_client),
        Arc::clone(&config_source),
        Arc::clone(&data_context),
        cancel.clone(),
    ));

    // ── SIP Server ────────────────────────────────────────────────────────────
    let _sip_server = SipServerBuilder::new(proxy_config)
        .with_cancel_token(cancel.clone())
        .with_skip_migrate(true)
        .with_data_context(Arc::clone(&data_context))
        .with_call_router(edge_router)
        .register_module("internal-peer", InternalPeerModule::create)
        .register_module("acl", AclModule::create)
        .register_module("auth", AuthModule::create)
        .register_module("call", CallModule::create)
        .build()
        .await?;

    info!("SIP edge ready on {}:{}", cfg.sip_addr, cfg.udp_port);

    signal::ctrl_c().await?;
    info!("shutdown");
    cancel.cancel();
    Ok(())
}

fn build_proxy_config(cfg: &EdgeConfig) -> ProxyConfig {
    ProxyConfig {
        addr: cfg.sip_addr.clone(),
        udp_port: Some(cfg.udp_port),
        tcp_port: if cfg.tcp_port > 0 { Some(cfg.tcp_port) } else { None },
        tls_port: if cfg.tls_port > 0 { Some(cfg.tls_port) } else { None },
        modules: Some(vec!["internal-peer".into(), "acl".into(), "auth".into(), "call".into()]),
        ..Default::default()
    }
}

/// Pull trunks/routes/acl from Control Plane and inject into ProxyDataContext.
///
/// Injection happens via the public ProxyConfig fields (`trunks`, `routes`)
/// followed by `reload_trunks` / `reload_routes` — no `set_*` methods needed.
async fn inject_grpc_config(source: &GrpcConfigSource, ctx: &Arc<ProxyDataContext>) {
    // ── Trunks ─────────────────────────────────────────────────────────────
    match source.load_trunks().await {
        Ok(trunks) => {
            let count = trunks.len();
            let mut config = (*ctx.config()).clone();
            config.trunks = trunks;
            if let Err(e) = ctx.reload_trunks(false, Some(Arc::new(config))).await {
                warn!(error = %e, "trunk reload after gRPC pull failed");
            }
            info!(count, "injected trunks into data context");
        }
        Err(e) => warn!(error = %e, "initial trunk pull failed"),
    }

    // ── Routes ─────────────────────────────────────────────────────────────
    match source.load_routes().await {
        Ok(routes) => {
            let count = routes.len();
            let mut config = (*ctx.config()).clone();
            config.routes = Some(routes);
            if let Err(e) = ctx.reload_routes(false, Some(Arc::new(config))).await {
                warn!(error = %e, "route reload after gRPC pull failed");
            }
            info!(count, "injected routes into data context");
        }
        Err(e) => warn!(error = %e, "initial route pull failed"),
    }

    // ── ACL ────────────────────────────────────────────────────────────────
    match source.load_acl_rules().await {
        Ok(rules) => {
            let count = rules.len();
            ctx.set_acl_rules(rules);
            info!(count, "injected acl rules into data context");
        }
        Err(e) => warn!(error = %e, "initial acl pull failed"),
    }
}
