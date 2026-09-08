//! Process-wide admission binding: one authority connection per agent
//! process, negotiated before the provider runtime is composed and before a
//! child announces socket readiness. Installed once; a changed policy needs a
//! process restart (P2 restart-only contract).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use super::client::{AuthorityConnection, ClientError};
use super::directory::AuthorityDirectory;
use crate::application::ports::{AttemptAdmission, Credential};
use crate::domain::inference_admission::WorkloadClass;
use crate::infrastructure::provider_runtime_admission::{
    AdmissionRuntimeContext, AdmissionRuntimeProposal,
};
use crate::infrastructure::providers::{SingleAttemptClient, default_client_builder};

const CONTEXT_FORMAT: u32 = 1;

/// Sidecar handed to a descendant (0600 file, never argv/env): where the
/// authority is and which pre-registered capability the child must bind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionContext {
    pub format: u32,
    pub endpoint: PathBuf,
    pub epoch: u64,
    pub serial: u64,
    pub token: String,
}

pub fn write_admission_context(
    path: &Path,
    endpoint: &Path,
    credential: &Credential,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let context = AdmissionContext {
        format: CONTEXT_FORMAT,
        endpoint: endpoint.to_path_buf(),
        epoch: credential.scope.epoch,
        serial: credential.scope.serial,
        token: credential.token.clone(),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&serde_json::to_vec(&context).expect("context is serializable"))?;
    file.sync_all()
}

pub fn read_admission_context(path: &Path) -> Result<AdmissionContext, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("admission context {} unreadable: {e}", path.display()))?;
    let context: AdmissionContext = serde_json::from_slice(&bytes)
        .map_err(|e| format!("admission context {} malformed: {e}", path.display()))?;
    if context.format != CONTEXT_FORMAT {
        return Err(format!(
            "admission context format {} unsupported (expected {CONTEXT_FORMAT})",
            context.format
        ));
    }
    Ok(context)
}

/// The installed binding. The runtime thread owns the connection's I/O so the
/// binding is independent of whichever runtime the agent loop uses.
pub struct ProcessAdmission {
    connection: Arc<AuthorityConnection>,
    context: Arc<AdmissionRuntimeContext>,
    proposal: AdmissionRuntimeProposal,
    client_dir: PathBuf,
    runtime: tokio::runtime::Runtime,
}

impl std::fmt::Debug for ProcessAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessAdmission")
            .field("client_dir", &self.client_dir)
            .finish_non_exhaustive()
    }
}

impl ProcessAdmission {
    pub fn connection(&self) -> &Arc<AuthorityConnection> {
        &self.connection
    }
    pub fn runtime_context(&self) -> &Arc<AdmissionRuntimeContext> {
        &self.context
    }
    pub fn proposal(&self) -> &AdmissionRuntimeProposal {
        &self.proposal
    }
    /// Directory containing the client socket; the only path a container
    /// child needs mounted.
    pub fn client_dir(&self) -> &Path {
        &self.client_dir
    }
    pub fn endpoint(&self) -> PathBuf {
        self.client_dir.join("admission.sock")
    }
}

static PROCESS: OnceLock<Arc<ProcessAdmission>> = OnceLock::new();

pub fn current() -> Option<Arc<ProcessAdmission>> {
    PROCESS.get().cloned()
}

/// How this process joins the authority.
#[derive(Debug, Clone)]
pub enum Negotiation {
    /// A top-level session: register a fresh interactive root at `directory`.
    Root { directory: PathBuf },
    /// A descendant: bind the capability its parent registered.
    Child { context: PathBuf },
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .thread_name("quecto-admission")
        .build()
        .map_err(|e| format!("admission runtime: {e}"))
}

async fn connect(negotiation: &Negotiation) -> Result<(AuthorityConnection, PathBuf), String> {
    match negotiation {
        Negotiation::Root { directory } => {
            // Clients only validate; they never create authority state.
            let dir = AuthorityDirectory::existing(directory).map_err(|e| {
                format!(
                    "admission authority unreachable: directory {} ({e}); start `quecto admission-broker run` or remove the admission section",
                    directory.display()
                )
            })?;
            let endpoint = dir.client_socket();
            let connection = AuthorityConnection::connect(&endpoint).await.map_err(|e| {
                format!(
                    "admission authority unreachable at {} ({e}); start `quecto admission-broker run` or remove the admission section",
                    endpoint.display()
                )
            })?;
            let credential = connection
                .register_root(WorkloadClass::Interactive)
                .await
                .map_err(|e| format!("admission root registration: {e}"))?;
            connection
                .bind(credential)
                .await
                .map_err(|e| format!("admission root bind: {e}"))?;
            Ok((connection, dir.client_dir()))
        }
        Negotiation::Child { context: path } => {
            let context = read_admission_context(path)?;
            // The sidecar is single-use whatever the outcome: capability
            // material never lingers on disk after the child has read it.
            let outcome = bind_child(context).await;
            let _ = std::fs::remove_file(path);
            outcome
        }
    }
}

async fn bind_child(context: AdmissionContext) -> Result<(AuthorityConnection, PathBuf), String> {
    let connection = AuthorityConnection::connect(&context.endpoint)
        .await
        .map_err(|e| format!("admission authority unreachable: {e}"))?;
    let credential = Credential {
        scope: crate::domain::inference_admission::ScopeId {
            epoch: context.epoch,
            serial: context.serial,
        },
        token: context.token,
    };
    connection
        .bind(credential)
        .await
        .map_err(|e| format!("admission capability rejected: {e}"))?;
    let client_dir = context
        .endpoint
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    Ok((connection, client_dir))
}

/// Negotiate with the authority and build a binding without installing it.
/// Only `install` touches process-wide state.
pub fn negotiate(negotiation: Negotiation) -> Result<ProcessAdmission, String> {
    let runtime = runtime()?;
    // Never block_on from inside a foreign runtime: negotiate on a plain thread.
    let handle = runtime.handle().clone();
    let negotiation_thread = negotiation.clone();
    let joined = std::thread::spawn(move || handle.block_on(connect(&negotiation_thread)))
        .join()
        .map_err(|_| "admission negotiation thread panicked".to_string())?;
    let (connection, client_dir) = joined?;
    let proposal = connection.hello().proposal.clone();
    let mut gates: BTreeMap<String, Arc<dyn AttemptAdmission>> = BTreeMap::new();
    for alias in proposal.policy.aliases.keys() {
        gates.insert(
            alias.clone(),
            connection
                .gate(alias)
                .map_err(|e| format!("admission gate for '{alias}': {e}"))?,
        );
    }
    let client = SingleAttemptClient::build(default_client_builder())
        .map_err(|e| format!("admission HTTP client: {e}"))?;
    let context = AdmissionRuntimeContext::new(proposal.clone(), gates, client)?;
    Ok(ProcessAdmission {
        connection: Arc::new(connection),
        context: Arc::new(context),
        proposal,
        client_dir,
        runtime,
    })
}

/// Negotiate and install the process binding. A binding is installed once per
/// process; later calls return it unchanged (policy is restart-only).
pub fn install(negotiation: Negotiation) -> Result<Arc<ProcessAdmission>, String> {
    install_in(&PROCESS, negotiation)
}

/// `install` against an explicit slot so the once-only semantics are testable
/// without touching the process-wide binding.
pub fn install_in(
    slot: &OnceLock<Arc<ProcessAdmission>>,
    negotiation: Negotiation,
) -> Result<Arc<ProcessAdmission>, String> {
    if let Some(existing) = slot.get() {
        return Ok(existing.clone());
    }
    let binding = Arc::new(negotiate(negotiation)?);
    match slot.set(binding.clone()) {
        Ok(()) => Ok(binding),
        Err(_) => Ok(slot.get().expect("set by a concurrent install").clone()),
    }
}

impl ProcessAdmission {
    /// Wait (bounded) for outstanding completions, then retire this scope.
    /// Returns whether completions drained and the retire outcome; a retire
    /// refused as busy is reported, never forced.
    pub fn shutdown(&self, limit: std::time::Duration) -> (bool, Result<(), ClientError>) {
        let handle = self.runtime.handle().clone();
        let connection = self.connection.clone();
        std::thread::spawn(move || {
            handle.block_on(async move {
                let drained = connection.drain(limit).await;
                let retired = connection.retire().await;
                (drained, retired)
            })
        })
        .join()
        .unwrap_or((false, Err(ClientError::Closed)))
    }
}

/// Orderly exit of the installed binding (no-op when none is installed).
pub fn shutdown(limit: std::time::Duration) {
    let Some(binding) = current() else {
        return;
    };
    match binding.shutdown(limit) {
        (true, Ok(())) => {}
        (drained, retired) => tracing::warn!(
            drained,
            ?retired,
            "admission shutdown left work unacknowledged; the authority keeps it as uncertain"
        ),
    }
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
