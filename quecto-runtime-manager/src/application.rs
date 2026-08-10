use crate::domain::{EnsureRuntimeRequest, RuntimeEnvelope, runtime_ref, validate_ensure_request};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Instant,
};
use thiserror::Error;
use tokio::process::Child;

#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub runtime_root: PathBuf,
    pub socket_root: PathBuf,
    pub api_port_base: u16,
    pub api_port_span: u16,
    pub max_runtimes: usize,
    pub system_prompt_path: PathBuf,
    pub seed_config_path: PathBuf,
    pub seed_credentials_path: PathBuf,
    pub mcp_url: Option<String>,
    pub mcp_allowlist: String,
    pub mcp_token_path: PathBuf,
    pub kubernetes_namespace: String,
    pub pod_image: String,
    pub pod_pull_secret: Option<String>,
    /// Name of the Kubernetes Secret holding the shared `credentials.json`.
    /// Runtime pods sync refreshed OAuth tokens back into this Secret so newly
    /// spawned pods start from a fresh, non-expired token.
    pub credentials_secret_name: String,
    /// In-cluster URL at which this manager is reachable by runtime pods (used
    /// as the credential sync callback target). E.g. `http://quecto-runtime-manager:8080`.
    pub manager_self_url: String,
    /// Bearer token runtime pods present when calling the credential sync
    /// endpoint. Same value as [`AppState::token`]; mirrored here so it can be
    /// injected into the pod manifest.
    pub manager_token: Option<String>,
}

#[derive(Debug)]
pub struct ManagedRuntime {
    pub runtime_ref: String,
    pub session_name: String,
    pub session_key: String,
    pub base_dir: PathBuf,
    pub socket_path: PathBuf,
    pub port: u16,
    pub agent: Option<Child>,
    pub api: Option<Child>,
    pub mcp: Option<Child>,
    pub pod_name: Option<String>,
    pub pod_ip: Option<String>,
    pub last_used_at: Instant,
}

impl ManagedRuntime {
    pub fn touch(&mut self) {
        self.last_used_at = Instant::now();
    }
}

#[derive(Debug, Default)]
pub struct RuntimeRegistry {
    runtimes: HashMap<String, ManagedRuntime>,
    used_ports: HashSet<u16>,
}

impl RuntimeRegistry {
    pub fn active_count(&self) -> usize {
        self.runtimes.len()
    }

    pub fn get(&self, runtime_ref: &str) -> Option<&ManagedRuntime> {
        self.runtimes.get(runtime_ref)
    }

    pub fn get_mut(&mut self, runtime_ref: &str) -> Option<&mut ManagedRuntime> {
        self.runtimes.get_mut(runtime_ref)
    }

    pub fn insert(&mut self, runtime: ManagedRuntime) {
        self.used_ports.insert(runtime.port);
        self.runtimes.insert(runtime.runtime_ref.clone(), runtime);
    }

    pub fn allocate_port(
        &mut self,
        config: &ManagerConfig,
        runtime_ref: &str,
    ) -> Result<u16, ManagerError> {
        for offset in 0..config.api_port_span {
            let port =
                config.api_port_base + ((stable_hash(runtime_ref) + offset) % config.api_port_span);
            if self.used_ports.insert(port) {
                return Ok(port);
            }
        }

        Err(ManagerError::NoAvailablePorts)
    }

    pub fn release_port(&mut self, port: u16) {
        self.used_ports.remove(&port);
    }

    #[cfg(test)]
    pub fn reserve_port_for_test(&mut self, port: u16) {
        self.used_ports.insert(port);
    }

    pub fn stop(&mut self, runtime_ref: &str) -> bool {
        self.stop_and_take_pod(runtime_ref).is_some()
    }

    pub fn stop_and_take_pod(&mut self, runtime_ref: &str) -> Option<Option<String>> {
        let mut runtime = self.runtimes.remove(runtime_ref)?;

        self.used_ports.remove(&runtime.port);
        for child in [&mut runtime.mcp, &mut runtime.api, &mut runtime.agent]
            .into_iter()
            .flatten()
        {
            let _ = child.start_kill();
        }
        let _ = std::fs::remove_file(&runtime.socket_path);
        Some(runtime.pod_name)
    }

    pub fn reap_one_oldest(&mut self) -> bool {
        self.reap_one_oldest_pod().is_some()
    }

    pub fn reap_one_oldest_pod(&mut self) -> Option<Option<String>> {
        let runtime_ref = self
            .runtimes
            .iter()
            .min_by_key(|(_, runtime)| runtime.last_used_at)
            .map(|(runtime_ref, _)| runtime_ref.clone())?;

        self.stop_and_take_pod(&runtime_ref)
    }
}

pub fn ensure_capacity(
    registry: &mut RuntimeRegistry,
    config: &ManagerConfig,
    incoming_ref: &str,
) -> Result<(), ManagerError> {
    if registry.active_count() < config.max_runtimes || registry.get(incoming_ref).is_some() {
        return Ok(());
    }

    if registry.reap_one_oldest_pod().is_some() {
        Ok(())
    } else {
        Err(ManagerError::RuntimeLimitReached)
    }
}

pub fn ensure_request_envelope(
    body: &EnsureRuntimeRequest,
) -> Result<RuntimeEnvelope, ManagerError> {
    validate_ensure_request(body).map_err(ManagerError::InvalidRequest)?;
    Ok(RuntimeEnvelope::running(runtime_ref(body)))
}

fn stable_hash(value: &str) -> u16 {
    let mut hash: u32 = 0;
    for byte in value.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }
    (hash & 0xffff) as u16
}

#[derive(Debug, Error)]
pub enum ManagerError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("runtime limit reached")]
    RuntimeLimitReached,
    #[error("no available runtime ports")]
    NoAvailablePorts,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("kubernetes api error: {0}")]
    KubernetesApi(u16),
    #[error("runtime failed health check")]
    RuntimeUnhealthy,
}

#[cfg(test)]
#[path = "application_tests.rs"]
mod tests;
