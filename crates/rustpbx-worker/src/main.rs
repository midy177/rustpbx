mod addons;
mod call_router;
mod cdr_hook;
mod config;
mod control_client;
mod headers;
mod internal_peer;
mod metrics;
mod proto;
mod rtp_gateway;
mod session_hook;
#[allow(dead_code)]
mod worker_call_module;

use crate::{
    addons::collect_addon_cdr_hooks,
    call_router::WorkerCallRouter,
    cdr_hook::GrpcCdrHook,
    config::WorkerConfig,
    control_client::{ControlClient, run_heartbeat},
    internal_peer::InternalPeerModule,
    internal_peer::init_trusted_edges,
    metrics::{WorkerMetrics, start_metrics_server},
    session_hook::ActiveCallTrackerHook,
};
use anyhow::Result;
use ipnetwork::IpNetwork;
use rustpbx::{
    call::RoutingState,
    callrecord::CallRecordManagerBuilder,
    config::{ProxyConfig, RtpConfig},
    proxy::{
        acl::AclModule,
        auth::AuthModule,
        call::CallModule,
        data::ProxyDataContext,
        presence::PresenceModule,
        registrar::RegistrarModule,
        server::SipServerBuilder,
    },
};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use tokio::{signal, sync::Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rustpbx-worker.toml".to_string());

    let cfg = if std::path::Path::new(&config_path).exists() {
        WorkerConfig::load(&config_path)?
    } else {
        WorkerConfig::default()
    };

    // ── Tracing ───────────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.log.as_str().into()),
        )
        .init();

    info!(
        worker_id = %cfg.worker_id,
        sip = format!("{}:{}", cfg.sip_addr, cfg.sip_port),
        rtp_external = ?cfg.rtp_external_ip,
        control_plane = %cfg.control_plane_addr,
        trusted_edges = cfg.trusted_edges.len(),
        "rustpbx-worker starting"
    );

    // ── Initialise trusted Edge list (OnceLock — must be before SipServer build) ─
    let trusted_networks: Vec<IpNetwork> = cfg
        .trusted_edges
        .iter()
        .filter_map(|s| match IpNetwork::from_str(s) {
            Ok(net) => Some(net),
            Err(e) => {
                warn!(entry = %s, error = %e, "invalid trusted_edges entry, skipping");
                None
            }
        })
        .collect();
    init_trusted_edges(trusted_networks);

    // ── Prometheus metrics ────────────────────────────────────────────────────
    if let Some(ref addr) = cfg.metrics_addr {
        start_metrics_server(addr).await?;
    }
    let worker_metrics = Arc::new(WorkerMetrics::new());

    // ── Connect to Control Plane & register ──────────────────────────────────
    let mut cp_client = ControlClient::connect(&cfg).await?;
    let active_calls = Arc::clone(&cp_client.active_calls);

    let ack = cp_client.register().await?;
    if !ack.accepted {
        anyhow::bail!("control plane rejected worker registration");
    }
    info!("registered with control plane");

    let cp_client = Arc::new(Mutex::new(cp_client));

    // ── CancellationToken ─────────────────────────────────────────────────────
    let cancel = CancellationToken::new();

    // ── Heartbeat loop ────────────────────────────────────────────────────────
    {
        let mut hb_client = ControlClient::connect(&cfg).await?;
        hb_client.active_calls = Arc::clone(&active_calls);
        tokio::spawn(run_heartbeat(hb_client, cfg.heartbeat_secs, cancel.clone()));
    }

    // ── CDR hooks (3 layers) ──────────────────────────────────────────────────
    //   1. Addon metrics (observability Prometheus counters/histograms)
    //   2. ActiveCallTracker — decrements active_calls counter on call end
    //   3. GrpcCdrHook — uploads CDR to Control Plane
    let grpc_hook: Box<dyn rustpbx::callrecord::CallRecordHook> =
        Box::new(GrpcCdrHook::new(Arc::clone(&cp_client), cfg.worker_id.clone()));
    let tracker_hook: Box<dyn rustpbx::callrecord::CallRecordHook> = Box::new(
        ActiveCallTrackerHook {
            active_calls: Arc::clone(&active_calls),
            metrics: Arc::clone(&worker_metrics),
        },
    );

    let mut cdr_builder = CallRecordManagerBuilder::new()
        .with_cancel_token(cancel.clone());

    for hook in collect_addon_cdr_hooks() {
        cdr_builder = cdr_builder.with_hook(hook);
    }
    cdr_builder = cdr_builder.with_hook(tracker_hook);
    cdr_builder = cdr_builder.with_hook(grpc_hook);

    let mut cdr_manager = cdr_builder.build().await?;
    let cdr_sender = cdr_manager.sender.clone();
    tokio::spawn(async move { cdr_manager.serve().await });

    // ── RTP config ────────────────────────────────────────────────────────────
    let rtp_config = RtpConfig {
        external_ip: cfg.rtp_external_ip.clone(),
        bind_ip: Some(cfg.rtp_bind_ip.clone()),
        start_port: Some(cfg.rtp_start_port),
        end_port: Some(cfg.rtp_end_port),
        ..Default::default()
    };

    // ── ProxyDataContext + RoutingState ───────────────────────────────────────
    let proxy_config = Arc::new(build_proxy_config(&cfg));
    let data_context = Arc::new(
        ProxyDataContext::new(proxy_config.clone(), None)
            .await
            .map_err(|e| anyhow::anyhow!("failed to init data context: {e}"))?,
    );
    let routing_state = Arc::new(RoutingState::new());

    // WorkerCallRouter: shares active_calls counter with ControlClient (for
    // heartbeat reporting) and ActiveCallTrackerHook (for decrement on CDR).
    let worker_router = WorkerCallRouter {
        data_context: Arc::clone(&data_context),
        rtp_config: rtp_config.clone(),
        routing_state: Arc::clone(&routing_state),
        active_calls: Arc::clone(&active_calls),
        metrics: Arc::clone(&worker_metrics),
        edge_sip_addr: cfg.edge_sip_addr.clone(),
    };

    // ── SIP Server ────────────────────────────────────────────────────────────
    // Module chain: internal-peer → acl → auth → registrar → presence → call
    //
    // Uses the main crate's CallModule (full B2BUA/IVR/Queue) with WorkerCallRouter
    // that decodes Edge-encoded routing decisions into Dialplan objects.
    // Media mode is Anchored (MediaProxyMode::All).
    let _sip_server = SipServerBuilder::new(proxy_config)
        .with_cancel_token(cancel.clone())
        .with_rtp_config(rtp_config)
        .with_callrecord_sender(Some(cdr_sender))
        .with_data_context(Arc::clone(&data_context))
        .with_call_router(Box::new(worker_router))
        .register_module("internal-peer", InternalPeerModule::create)
        .register_module("acl", AclModule::create)
        .register_module("auth", AuthModule::create)
        .register_module("registrar", RegistrarModule::create)
        .register_module("presence", PresenceModule::create)
        .register_module("call", CallModule::create)
        .build()
        .await?;

    info!("media worker ready — SIP on {}:{}", cfg.sip_addr, cfg.sip_port);

    // ── Graceful shutdown ─────────────────────────────────────────────────────
    signal::ctrl_c().await?;
    info!("shutdown signal received — draining calls");
    cancel.cancel();

    let _ = tokio::time::timeout(
        tokio::time::Duration::from_secs(30),
        wait_for_drain(&active_calls),
    )
    .await;

    info!("worker stopped");
    Ok(())
}

fn build_proxy_config(cfg: &WorkerConfig) -> ProxyConfig {
    let mut config = ProxyConfig {
        addr: cfg.sip_addr.clone(),
        udp_port: Some(cfg.sip_port),
        ..Default::default()
    };
    config.modules = Some(vec![
        "internal-peer".into(),
        "acl".into(),
        "auth".into(),
        "registrar".into(),
        "presence".into(),
        "call".into(),
    ]);
    config
}

async fn wait_for_drain(active: &AtomicU32) {
    use std::sync::atomic::Ordering;
    loop {
        if active.load(Ordering::Relaxed) == 0 {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
}
