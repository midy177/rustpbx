/// Selects the best available Media Worker for an incoming call.
///
/// Queries the Control Plane for the current worker list on each call
/// (list is small and cached server-side).  Selects the worker with the
/// most available capacity (max_concurrent - active_calls).
use crate::grpc_client::GrpcControlClient;
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WorkerEndpoint {
    pub worker_id: String,
    /// Internal SIP contact: sip:<ip>:<port>
    pub sip_contact: String,
    pub available_capacity: u32,
    /// EdgeWorker gRPC addr (host:port) for AllocateCall; empty if the worker
    /// doesn't serve it (Edge then forwards without reservation).
    pub edge_worker_addr: String,
}

pub struct WorkerSelector {
    client: Arc<RwLock<GrpcControlClient>>,
    required_labels: HashMap<String, String>,
    required_capabilities: Vec<String>,
}

impl WorkerSelector {
    pub fn new(
        client: Arc<RwLock<GrpcControlClient>>,
        required_labels: HashMap<String, String>,
        required_capabilities: Vec<String>,
    ) -> Self {
        Self {
            client,
            required_labels,
            required_capabilities,
        }
    }

    /// Query Control Plane and return the best worker for a new call.
    pub async fn select(&self, tenant_id: Option<i64>) -> Result<WorkerEndpoint> {
        let mut client = self.client.write().await;
        let list = client
            .get_available_workers(
                tenant_id,
                self.required_labels.clone(),
                self.required_capabilities.clone(),
            )
            .await?;

        if list.workers.is_empty() {
            return Err(anyhow!(
                "no available workers matching labels {:?} and capabilities {:?}",
                self.required_labels,
                self.required_capabilities
            ));
        }

        // Workers are already sorted by available capacity (Control Plane does this).
        let best = &list.workers[0];
        let capacity = best.max_concurrent.saturating_sub(best.active_calls);

        debug!(
            worker_id = %best.worker_id,
            sip_addr = %best.sip_addr,
            active = best.active_calls,
            max = best.max_concurrent,
            labels = ?best.labels,
            capabilities = ?best.capabilities,
            "selected worker"
        );

        Ok(WorkerEndpoint {
            worker_id: best.worker_id.clone(),
            sip_contact: format!("sip:{}", best.sip_addr),
            available_capacity: capacity,
            edge_worker_addr: best.edge_worker_addr.clone(),
        })
    }
}
