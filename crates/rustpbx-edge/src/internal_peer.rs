//! `InternalPeerModule` — Edge-side module that recognises internal INVITEs
//! from trusted Worker instances and pre-populates the `TransactionCookie`
//! with `TrunkContext` + `InternalCallContext` so downstream modules (ACL,
//! Auth) skip their normal checks.
//!
//! Runs first in the module chain (before AclModule).
//!
//! Trusted Worker IP/CIDR list is stored in a process-global `OnceLock`
//! because the module factory only accepts `(SipServerRef, Arc<ProxyConfig>)`.

use crate::headers::decode_headers;
use anyhow::Result;
use async_trait::async_trait;
use ipnetwork::IpNetwork;
use rsipstack::sip::Method;
use rsipstack::sip::prelude::HeadersExt;
use rsipstack::transaction::transaction::Transaction;
use rsipstack::transport::SipConnection;
use rustpbx::call::cookie::{TrunkContext, TransactionCookie};
use rustpbx::config::ProxyConfig;
use rustpbx::proxy::server::SipServerRef;
use rustpbx::proxy::{ProxyAction, ProxyModule};
use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

static TRUSTED_WORKERS: OnceLock<Vec<IpNetwork>> = OnceLock::new();

pub fn init_trusted_workers(workers: Vec<IpNetwork>) {
    if let Err(existing) = TRUSTED_WORKERS.set(workers) {
        warn!(count = existing.len(), "trusted_workers already initialised — ignoring re-init");
    }
}

pub struct InternalPeerModule {
    trusted_networks: Vec<IpNetwork>,
}

impl InternalPeerModule {
    pub fn new(trusted: Vec<IpNetwork>) -> Self {
        Self { trusted_networks: trusted }
    }

    pub fn create(
        _server: SipServerRef,
        _config: Arc<ProxyConfig>,
    ) -> Result<Box<dyn ProxyModule>> {
        let trusted = TRUSTED_WORKERS
            .get()
            .cloned()
            .unwrap_or_default();
        Ok(Box::new(InternalPeerModule::new(trusted)))
    }
}

#[async_trait]
impl ProxyModule for InternalPeerModule {
    fn name(&self) -> &str {
        "internal-peer"
    }

    fn allow_methods(&self) -> Vec<Method> {
        vec![Method::Invite]
    }

    async fn on_start(&mut self) -> Result<()> {
        debug!(count = self.trusted_networks.len(), "edge internal-peer module started");
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        Ok(())
    }

    async fn on_transaction_begin(
        &self,
        _token: CancellationToken,
        tx: &mut Transaction,
        cookie: TransactionCookie,
    ) -> Result<ProxyAction> {
        if tx.original.method != Method::Invite {
            return Ok(ProxyAction::Continue);
        }

        let source_ip = match extract_source_ip(tx) {
            Some(ip) => ip,
            None => return Ok(ProxyAction::Continue),
        };

        let is_trusted = self
            .trusted_networks
            .iter()
            .any(|net| net.contains(source_ip));

        if !is_trusted {
            return Ok(ProxyAction::Continue);
        }

        let internal_ctx = match decode_headers(&tx.original.headers) {
            Some(ctx) => ctx,
            None => {
                debug!(
                    %source_ip,
                    "trusted worker source but no X-Route-Action header — treating as normal call"
                );
                return Ok(ProxyAction::Continue);
            }
        };

        let trunk_ctx = TrunkContext {
            id: internal_ctx.trunk_id,
            name: internal_ctx.trunk_name.clone(),
            tenant_id: internal_ctx.tenant_id,
            did_numbers: Vec::new(),
        };
        cookie.insert_extension(trunk_ctx);
        cookie.insert_extension(internal_ctx);

        debug!("internal INVITE from trusted worker — context injected");
        Ok(ProxyAction::Continue)
    }
}

fn extract_source_ip(tx: &Transaction) -> Option<IpAddr> {
    let via = tx.original.via_header().ok()?;
    let (_, target) = SipConnection::parse_target_from_via(via).ok()?;
    target.host.try_into().ok()
}
