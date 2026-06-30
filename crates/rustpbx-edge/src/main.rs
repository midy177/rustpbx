mod active_call_hook;
mod call_router;
mod config;
mod config_source;
mod config_watcher;
mod dialog_path;
mod edge_worker_server;
mod grpc_client;
mod headers;
mod internal_peer;
mod proto;
mod worker_selector;

use crate::{
    active_call_hook::EdgeActiveCallHook,
    call_router::EdgeCallRouter,
    config::EdgeConfig,
    config_source::{ConfigSource, GrpcConfigSource},
    config_watcher::run_config_watcher,
    dialog_path::{DialogPathModule, build_record_route, init_record_route},
    grpc_client::GrpcControlClient,
    internal_peer::InternalPeerModule,
    internal_peer::init_trusted_workers,
    worker_selector::WorkerSelector,
};
use anyhow::Result;
use ipnetwork::IpNetwork;
use rustpbx::{
    call::RoutingState,
    callrecord::CallRecordManagerBuilder,
    config::ProxyConfig,
    proxy::{
        acl::AclModule, auth::AuthModule, call::CallModule, data::ProxyDataContext,
        server::SipServerBuilder,
    },
};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rustpbx-edge.toml".to_string());

    let mut cfg = if std::path::Path::new(&config_path).exists() {
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

    // ── Health endpoint (started early, before any blocking connect) ──────────
    // /healthz (liveness) responds immediately; /readyz flips to 200 once the
    // edge registers with the control plane.
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Some(ref h) = cfg.health_addr {
        let addr: std::net::SocketAddr = h.parse()?;
        let ready = Arc::clone(&ready);
        tokio::spawn(async move {
            if let Err(e) = rustpbx_netprobe::health::serve(addr, ready).await {
                tracing::error!(error = %e, "health server exited");
            }
        });
    }

    // ── Connect to Control Plane ──────────────────────────────────────────────
    // Load client TLS once (CA verify + optional client cert for mutual TLS);
    // None → plaintext. Reused for every Control Plane connection below.
    let cp_tls = cfg.tls.load()?;
    if cp_tls.is_some() {
        let mutual = !cfg.tls.client_cert_path.trim().is_empty();
        info!(mutual, "control plane TLS enabled");
    }
    let grpc_client = GrpcControlClient::connect_with_retry(
        &cfg.control_plane_addr,
        cfg.edge_id.clone(),
        cp_tls.as_ref(),
    )
    .await?;
    let grpc_client = Arc::new(tokio::sync::RwLock::new(grpc_client));

    // ── GrpcConfigSource ──────────────────────────────────────────────────────
    let config_source = GrpcConfigSource::new(
        GrpcControlClient::connect_with_retry(
            &cfg.control_plane_addr,
            cfg.edge_id.clone(),
            cp_tls.as_ref(),
        )
        .await?,
        None,
    );

    // ── Worker selector ───────────────────────────────────────────────────────
    let worker_selector = Arc::new(WorkerSelector::new(
        Arc::clone(&grpc_client),
        cfg.worker_required_labels.clone(),
        cfg.worker_required_capabilities.clone(),
    ));

    // ── CancellationToken ─────────────────────────────────────────────────────
    let cancel = CancellationToken::new();

    // Live in-flight call gauge: incremented by EdgeCallRouter on a successful
    // route, decremented by EdgeActiveCallHook on CDR completion, reported in
    // the heartbeat below.
    let active_calls = Arc::new(AtomicU32::new(0));

    // ── Detect public IP + NAT type (STUN) ────────────────────────────────────
    // Prefer the centrally-managed STUN list (superadmin → platform settings);
    // fall back to the node's local config when none is configured/reachable.
    let central_stun =
        grpc_client::fetch_platform_stun(&cfg.control_plane_addr, cp_tls.as_ref()).await;
    let stun = if central_stun.is_empty() {
        cfg.stun_servers.clone()
    } else {
        central_stun
    };
    let nat = rustpbx_netprobe::probe(&stun, std::time::Duration::from_secs(3)).await;
    info!(nat_type = %nat.nat_type, public_ip = ?nat.public_ip, "NAT probe complete");
    if cfg.public_ip.is_none()
        && let Some(ip) = nat.public_ip.clone()
    {
        info!(%ip, "using STUN-detected public IP as the edge public IP");
        cfg.public_ip = Some(ip);
    }
    let record_route = build_record_route(
        cfg.public_ip.as_deref().unwrap_or(&cfg.sip_addr),
        cfg.udp_port,
    );
    if record_route.is_none() {
        warn!("edge Record-Route disabled because no public SIP address is known");
    }
    init_record_route(record_route);

    // ── Register with Control Plane + heartbeat (observability only) ──────────
    // Edges aren't load-selected; this just lets the admin console see which
    // edges are alive, their address/version/region, and health.
    {
        let mut reg_client = GrpcControlClient::connect(
            &cfg.control_plane_addr,
            cfg.edge_id.clone(),
            cp_tls.as_ref(),
        )
        .await?;
        let info = build_edge_info(&cfg, &nat.nat_type);
        match reg_client.register_edge(info.clone()).await {
            Ok(_) => {
                info!(edge_id = %cfg.edge_id, "registered with control plane");
                ready.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => warn!(error = %e, "edge registration failed (will retry via heartbeat)"),
        }
        // Heartbeat loop: refresh liveness, re-register if the control plane
        // forgot us (e.g. it restarted).
        let hb_secs = cfg.heartbeat_secs.max(1);
        let ct = cancel.clone();
        let hb_active = Arc::clone(&active_calls);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(hb_secs));
            loop {
                tokio::select! {
                    _ = ct.cancelled() => break,
                    _ = ticker.tick() => {
                        let n = hb_active.load(Ordering::Relaxed);
                        match reg_client.edge_heartbeat(n).await {
                            Ok(true) => {}
                            Ok(false) => {
                                if let Err(e) = reg_client.register_edge(info.clone()).await {
                                    warn!(error = %e, "edge re-register failed");
                                }
                            }
                            Err(e) => warn!(error = %e, "edge heartbeat failed"),
                        }
                    }
                }
            }
        });
    }

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

    // ── CDR manager (edge-local) — drives the active-call gauge ───────────────
    // The CallModule emits a CDR per proxied call; the only hook here decrements
    // `active_calls` on completion. The authoritative CDR is reported by the
    // worker, so the edge uploads nothing.
    let mut cdr_builder = CallRecordManagerBuilder::new().with_cancel_token(cancel.clone());
    let active_hook: Box<dyn rustpbx::callrecord::CallRecordHook> = Box::new(EdgeActiveCallHook {
        active_calls: Arc::clone(&active_calls),
    });
    cdr_builder = cdr_builder.with_hook(active_hook);
    let mut cdr_manager = cdr_builder.build().await?;
    let cdr_sender = cdr_manager.sender.clone();
    tokio::spawn(async move { cdr_manager.serve().await });

    // ── EdgeCallRouter ────────────────────────────────────────────────────────
    let edge_router = Box::new(EdgeCallRouter::new(
        Arc::clone(&worker_selector),
        Arc::clone(&data_context),
        Arc::clone(&routing_state),
        cfg.edge_id.clone(),
        Arc::clone(&active_calls),
        Arc::clone(&grpc_client),
    ));

    // ── Config watcher (background, re-pulls + re-injects on change) ──────────
    let config_source = Arc::new(config_source);
    tokio::spawn(run_config_watcher(
        Arc::clone(&grpc_client),
        Arc::clone(&config_source),
        Arc::clone(&data_context),
        cancel.clone(),
    ));

    // ── EdgeWorker gRPC server (CallStateUpdate receiver) ─────────────────────
    if let Some(ref ew_addr) = cfg.edge_worker_addr {
        let ew_addr: std::net::SocketAddr = ew_addr.parse()?;
        let ct = cancel.clone();
        tracing::info!(%ew_addr, "edge-worker gRPC (CallStateUpdate) listening");
        tokio::spawn(async move {
            let svc = rustpbx_proto::edge::edge_worker_server::EdgeWorkerServer::new(
                edge_worker_server::EdgeWorkerServer,
            );
            let res = tonic::transport::Server::builder()
                .add_service(svc)
                .serve_with_shutdown(ew_addr, async move { ct.cancelled().await })
                .await;
            if let Err(e) = res {
                tracing::error!(error = %e, "edge-worker server exited");
            }
        });
    }

    // ── SIP Server ────────────────────────────────────────────────────────────
    let sip_server = SipServerBuilder::new(proxy_config)
        .with_cancel_token(cancel.clone())
        .with_skip_migrate(true)
        .with_callrecord_sender(Some(cdr_sender))
        .with_data_context(Arc::clone(&data_context))
        .with_call_router(edge_router)
        .register_module("internal-peer", InternalPeerModule::create)
        .register_module("acl", AclModule::create)
        .register_module("auth", AuthModule::create)
        .register_module("call", CallModule::create)
        .register_module("dialog-path", DialogPathModule::create)
        .build()
        .await?;

    info!("SIP edge ready on {}:{}", cfg.sip_addr, cfg.udp_port);

    // Cancel on Ctrl-C; serving returns when the token is cancelled.
    let shutdown = cancel.clone();
    tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
        info!("shutdown signal received");
        shutdown.cancel();
    });

    // Serve the SIP endpoint: this drives the transport listeners (binding the
    // TCP listener so the Edge accepts Worker connections) and processes
    // incoming SIP. Without this the server only binds UDP and never runs.
    if let Err(e) = sip_server.serve().await {
        tracing::error!(error = %e, "sip server exited with error");
    }
    Ok(())
}

/// Build the EdgeInfo reported to the Control Plane (for the admin console).
fn build_edge_info(cfg: &EdgeConfig, nat_type: &str) -> crate::proto::control::EdgeInfo {
    let host = cfg
        .public_ip
        .clone()
        .unwrap_or_else(|| cfg.sip_addr.clone());
    // The Edge always listens on TCP too (build_proxy_config sets tcp_port =
    // tcp_port>0 ? tcp_port : udp_port), so it's always advertised. TLS is
    // optional (only when tls_port > 0).
    let mut transports = vec!["udp".to_string(), "tcp".to_string()];
    if cfg.tls_port > 0 {
        transports.push("tls".to_string());
    }
    crate::proto::control::EdgeInfo {
        edge_id: cfg.edge_id.clone(),
        public_ip: cfg.public_ip.clone().unwrap_or_default(),
        sip_addr: format!("{host}:{}", cfg.udp_port),
        transports,
        region: cfg.region.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        active_calls: 0,
        nat_type: nat_type.to_string(),
    }
}

fn build_proxy_config(cfg: &EdgeConfig) -> ProxyConfig {
    ProxyConfig {
        addr: cfg.sip_addr.clone(),
        udp_port: Some(cfg.udp_port),
        // Always listen on TCP so Workers can reach the Edge over a persistent
        // TCP connection (worker→edge outbound + in-dialog). Defaults to the
        // main SIP port unless an explicit tcp_port is configured.
        tcp_port: Some(if cfg.tcp_port > 0 {
            cfg.tcp_port
        } else {
            cfg.udp_port
        }),
        tls_port: if cfg.tls_port > 0 {
            Some(cfg.tls_port)
        } else {
            None
        },
        modules: Some(vec![
            "internal-peer".into(),
            "acl".into(),
            "auth".into(),
            "call".into(),
            "dialog-path".into(),
        ]),
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
