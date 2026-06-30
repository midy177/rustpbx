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
    pub async fn select(
        &self,
        tenant_id: Option<i64>,
        affinity_key: Option<String>,
    ) -> Result<WorkerEndpoint> {
        let mut workers = self.select_all(tenant_id, affinity_key).await?;
        workers
            .drain(..)
            .next()
            .ok_or_else(|| anyhow!("control plane returned no available workers"))
    }

    /// Query Control Plane and return all matching workers in scheduling order.
    pub async fn select_all(
        &self,
        tenant_id: Option<i64>,
        affinity_key: Option<String>,
    ) -> Result<Vec<WorkerEndpoint>> {
        let mut client = self.client.write().await;
        let list = client
            .get_available_workers(
                tenant_id,
                self.required_labels.clone(),
                self.required_capabilities.clone(),
                affinity_key.clone(),
            )
            .await?;

        if list.workers.is_empty() {
            return Err(anyhow!(
                "no available workers matching labels {:?}, capabilities {:?}, affinity {:?}",
                self.required_labels,
                self.required_capabilities,
                affinity_key
            ));
        }

        // Workers are already sorted by Control Plane.
        Ok(list
            .workers
            .into_iter()
            .map(|worker| {
                let capacity = worker.max_concurrent.saturating_sub(worker.active_calls);
                debug!(
                    worker_id = %worker.worker_id,
                    sip_addr = %worker.sip_addr,
                    active = worker.active_calls,
                    max = worker.max_concurrent,
                    labels = ?worker.labels,
                    capabilities = ?worker.capabilities,
                    affinity_key = ?affinity_key,
                    "selected worker"
                );
                WorkerEndpoint {
                    worker_id: worker.worker_id,
                    sip_contact: format!("sip:{}", worker.sip_addr),
                    available_capacity: capacity,
                    edge_worker_addr: worker.edge_worker_addr,
                }
            })
            .collect())
    }
}
