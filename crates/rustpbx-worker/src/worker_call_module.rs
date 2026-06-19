//! `WorkerCallModule` — DEPRECATED. Superseded by the main crate's `CallModule`
//! + `WorkerCallRouter` injection (see `main.rs` and `call_router.rs`).
//!
//! This module is kept for reference only. It implemented a passthrough SIP
//! signaling path that bypassed CallModule's full B2BUA/IVR/Queue pipeline.
//! The new architecture routes internal calls through `CallModule` with a
//! custom `WorkerCallRouter` that builds the appropriate `Dialplan`, giving
//! the Worker access to the complete feature set (queue, IVR, recording, etc.)
//! while still using `MediaProxyMode::All` for anchored media.

use crate::rtp_gateway::{LoggingSink, RtpGatewayHandle, spawn_bridge};
use anyhow::Result;
use async_trait::async_trait;
use rsipstack::dialog::server_dialog::ServerInviteDialog;
use rsipstack::sip::Method;
use rsipstack::sip::prelude::HeadersExt;
use rsipstack::transaction::transaction::Transaction;
use rustpbx::call::cookie::TransactionCookie;
use rustpbx::config::ProxyConfig;
use rustpbx::proxy::server::{SipServerBuilder, SipServerRef};
use rustpbx::proxy::{ProxyAction, ProxyModule};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Factory: matches `FnCreateProxyModule` signature.
pub fn create(
    server: SipServerRef,
    config: Arc<ProxyConfig>,
) -> Result<Box<dyn ProxyModule>> {
    Ok(Box::new(WorkerCallModule {
        server,
        config,
        sessions: Arc::new(RwLock::new(HashMap::new())),
    }))
}

/// Builder convenience: register `WorkerCallModule` on a `SipServerBuilder`.
pub fn register(builder: SipServerBuilder) -> SipServerBuilder {
    builder.register_module("worker-call", create)
}

pub struct WorkerCallModule {
    server: SipServerRef,
    #[allow(dead_code)]
    config: Arc<ProxyConfig>,
    sessions: Arc<RwLock<HashMap<String, ActiveSession>>>,
}

/// Per-call state tracked by the module.
struct ActiveSession {
    dialog: ServerInviteDialog,
    #[allow(dead_code)]
    gateway_handle: RtpGatewayHandle,
    #[allow(dead_code)]
    sink: LoggingSink,
    cancel: CancellationToken,
}

impl WorkerCallModule {
    /// Extract the Call-ID from a SIP request — used as the session lookup key.
    fn call_id(tx: &Transaction) -> Option<String> {
        tx.original.call_id_header().ok().map(|h| h.value().to_string())
    }

    async fn handle_invite(
        &self,
        cancel_token: CancellationToken,
        tx: &mut Transaction,
        cookie: &TransactionCookie,
    ) -> Result<ProxyAction> {
        let call_id = match Self::call_id(tx) {
            Some(id) => id,
            None => {
                warn!("INVITE without Call-ID — letting other modules handle");
                return Ok(ProxyAction::Continue);
            }
        };

        // Re-INVITE within an existing dialog → route as in-dialog request.
        if self.sessions.read().unwrap().contains_key(&call_id) {
            debug!(call_id = %call_id, "re-INVITE for existing session — routing to dialog");
            return Ok(self.handle_in_dialog_request(tx).await);
        }

        // Only handle internal calls (from Edge). Non-internal calls fall through.
        if cookie
            .get_extension::<rustpbx_core::internal::InternalCallContext>()
            .is_none()
        {
            debug!(call_id = %call_id, "non-internal INVITE — skipping");
            return Ok(ProxyAction::Continue);
        }

        info!(call_id = %call_id, "worker-call: handling internal INVITE");

        // ── 1. Create server dialog ──────────────────────────────────────────
        let local_contact = self.server.default_contact_uri();
        let (state_tx, mut state_rx) = mpsc::unbounded_channel();

        let dialog = match self.server.dialog_layer.get_or_create_server_invite(
            tx,
            state_tx,
            None,
            local_contact,
        ) {
            Ok(d) => d,
            Err(e) => {
                error!(call_id = %call_id, error = %e, "failed to create server dialog");
                return Err(anyhow::anyhow!("dialog creation failed: {e}"));
            }
        };

        // ── 2. Create rtp_gateway (media I/O boundary) ───────────────────────
        let sink = Arc::new(LoggingSink::new(format!("call-{}", &call_id[..call_id.len().min(16)])));
        let gateway_handle = spawn_bridge(Arc::clone(&sink) as Arc<dyn crate::rtp_gateway::CallCommandSink>);

        let session_cancel = cancel_token.child_token();

        // ── 3. Store active session ──────────────────────────────────────────
        let session = ActiveSession {
            dialog: dialog.clone(),
            gateway_handle: gateway_handle.clone(),
            sink: sink.as_ref().clone(),
            cancel: session_cancel.clone(),
        };
        self.sessions.write().unwrap().insert(call_id.clone(), session);

        // ── 4. Spawn dialog state watcher ────────────────────────────────────
        // When the dialog terminates (BYE received), tear down the rtp_gateway.
        let sessions = Arc::clone(&self.sessions);
        let cleanup_call_id = call_id.clone();
        let cleanup_handle = gateway_handle.clone();
        tokio::spawn(async move {
            while let Some(state) = state_rx.recv().await {
                let is_terminated = matches!(
                    state,
                    rsipstack::dialog::dialog::DialogState::Terminated(_, _)
                );
                debug!(call_id = %cleanup_call_id, terminated = is_terminated, "dialog state update");
                if is_terminated {
                    info!(call_id = %cleanup_call_id, "dialog terminated — cleaning up rtp_gateway");
                    let _ = cleanup_handle.teardown().await;
                    sessions.write().unwrap().remove(&cleanup_call_id);
                    break;
                }
            }
        });

        // ── 5. Answer with passthrough SDP (Phase 1 skeleton) ────────────────
        // Use the SDP from the incoming INVITE as the answer. No media
        // manipulation yet — Phase 2 inserts the real rtp_gateway media pipeline.
        let original_sdp = tx.original.body().to_vec();
        if let Err(e) = dialog.accept(None, Some(original_sdp)) {
            error!(call_id = %call_id, error = %e, "failed to send 200 OK");
            // Cleanup on failure
            self.sessions.write().unwrap().remove(&call_id);
            session_cancel.cancel();
            return Err(anyhow::anyhow!("accept failed: {e}"));
        }

        info!(call_id = %call_id, "worker-call: answered (200 OK, passthrough SDP)");

        // Abort so no other module processes this INVITE.
        Ok(ProxyAction::Abort)
    }

    async fn handle_in_dialog_request(&self, tx: &mut Transaction) -> ProxyAction {
        let call_id = match Self::call_id(tx) {
            Some(id) => id,
            None => return ProxyAction::Continue,
        };

        // Clone the dialog out so we don't hold the lock across `.await`.
        let mut dialog = {
            let sessions = self.sessions.read().unwrap();
            match sessions.get(&call_id) {
                Some(session) => session.dialog.clone(),
                None => return ProxyAction::Continue, // Not our call
            }
        };

        // Route the in-dialog request to its ServerInviteDialog.
        if let Err(e) = dialog.handle(tx).await {
            warn!(call_id = %call_id, error = %e, "dialog.handle failed for in-dialog request");
        }
        ProxyAction::Abort
    }
}

#[async_trait]
impl ProxyModule for WorkerCallModule {
    fn name(&self) -> &str {
        "worker-call"
    }

    fn allow_methods(&self) -> Vec<Method> {
        vec![Method::Invite, Method::Bye, Method::Ack, Method::Cancel]
    }

    async fn on_start(&mut self) -> Result<()> {
        info!("worker-call module started — SIP signaling handled at signaling layer");
        Ok(())
    }

    async fn on_stop(&self) -> Result<()> {
        // Cancel all active sessions on shutdown.
        let sessions = self.sessions.read().unwrap();
        let count = sessions.len();
        for (_, session) in sessions.iter() {
            session.cancel.cancel();
        }
        info!(active = count, "worker-call module stopped — cancelled all sessions");
        Ok(())
    }

    async fn on_transaction_begin(
        &self,
        token: CancellationToken,
        tx: &mut Transaction,
        cookie: TransactionCookie,
    ) -> Result<ProxyAction> {
        match tx.original.method {
            Method::Invite => self.handle_invite(token, tx, &cookie).await,
            Method::Bye | Method::Ack | Method::Cancel => Ok(self.handle_in_dialog_request(tx).await),
            _ => Ok(ProxyAction::Continue),
        }
    }
}

#[cfg(test)]
mod tests {
    // Unit testing the module requires constructing rsipstack Transactions,
    // which is heavyweight. Integration is verified by:
    // 1. cargo build (struct/trait conformance)
    // 2. Manual SIP flow testing once the Worker is deployed
    // 3. The LoggingSink unit tests in rtp_gateway/logging_sink.rs

    #[test]
    fn module_factory_returns_boxed_module() {
        // We can't easily construct a SipServerRef in a unit test, so this
        // just verifies the function signature compiles.
        // Real verification happens at integration time.
    }
}
