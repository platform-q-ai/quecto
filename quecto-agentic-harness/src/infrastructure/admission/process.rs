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
use crate::application::{
    admission::dto::broker::Credential,
    admission::ports::inference_attempt::AttemptAdmission,
    admission::ports::{AuthorityLaunch, AuthoritySession, JoinedAuthority, RejoinRefusal},
};
use crate::domain::admission::value_objects::inference_admission::WorkloadClass;
use crate::infrastructure::providers::routing::provider_runtime_admission::{
    AdmissionRuntimeContext, AdmissionRuntimeProposal,
};
use crate::infrastructure::providers::transport::single_attempt_client::{
    SingleAttemptClient, default_client_builder,
};

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
    /// The reconnecting link every gate routes through (#2024 S3): a root
    /// re-registers after a broker restart/reset; a child fails closed.
    link: Arc<super::link::AuthorityLink>,
    context: Arc<AdmissionRuntimeContext>,
    proposal: AdmissionRuntimeProposal,
    client_dir: PathBuf,
    recorder: Arc<super::observed_gate::AdmissionRecorder>,
    runtime: tokio::runtime::Runtime,
    /// How this process joined: a child inherits the parent's authority and
    /// never validates its own config against the published policy (#2024 S3,
    /// #2023); a root validates its reload candidate.
    kind: BindingKind,
    /// The authority directory this process is bound to (root: its own; child:
    /// derived from the mounted client directory), surfaced in `get_state`.
    directory: PathBuf,
    /// What a root's broker serves, as startup negotiation resolved it, so a
    /// restart (#2615) starts the same broker again.
    launch: OnceLock<AuthorityLaunch>,
}

/// Whether this process owns the authority connection as a root or inherits it
/// from a parent as a child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    Root,
    Child,
}

impl std::fmt::Debug for ProcessAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessAdmission")
            .field("client_dir", &self.client_dir)
            .finish_non_exhaustive()
    }
}

impl ProcessAdmission {
    /// The current authority connection (whatever a reconnection last
    /// installed). Cloned per call so callers never hold a stale one.
    pub fn connection(&self) -> Arc<AuthorityConnection> {
        self.link.connection()
    }
    /// Register a descendant through the link (#2024 S3): a root whose broker
    /// restarted or was reset re-registers first; a child fails closed.
    pub async fn register_child(&self) -> Result<Credential, ClientError> {
        self.link.register_child().await
    }
    pub fn runtime_context(&self) -> &Arc<AdmissionRuntimeContext> {
        &self.context
    }
    pub fn proposal(&self) -> &AdmissionRuntimeProposal {
        &self.proposal
    }
    /// Whether this process inherits the authority from a parent (a child) and
    /// so must never validate its own config against the published policy.
    pub fn inherits_authority(&self) -> bool {
        self.kind == BindingKind::Child
    }
    pub fn kind(&self) -> BindingKind {
        self.kind
    }
    /// The authority directory this process is bound to.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// The authority epoch this process's current capability was minted in
    /// (live: a re-registration after `reset` moves it).
    pub fn epoch(&self) -> u64 {
        self.link.epoch()
    }
    /// Whether the authority link is open *and* holds a live capability.
    pub fn connected(&self) -> bool {
        self.link.connected()
    }
    /// Live health of the authority link (#2024 S3).
    pub fn health(&self) -> super::link::LinkHealth {
        self.link.health()
    }
    /// Whether the broker came back with a different policy, so only a
    /// session restart adopts it (#2615).
    pub fn restart_required(&self) -> bool {
        self.link.restart_required()
    }
    /// Be told whenever the link's health may have changed (loss, revocation,
    /// reconnection, exhaustion), so a projection can push it live.
    pub fn on_authority_change(&self, hook: super::link::LinkChangeHook) {
        self.link.on_change(hook);
    }
    #[cfg(test)]
    pub fn set_reconnect_backoff_for_test(
        &self,
        attempts: u32,
        base: std::time::Duration,
        max: std::time::Duration,
    ) {
        self.link.set_backoff_for_test(attempts, base, max);
    }
    /// Directory containing the client socket; the only path a container
    /// child needs mounted.
    pub fn client_dir(&self) -> &Path {
        &self.client_dir
    }
    pub fn endpoint(&self) -> PathBuf {
        self.client_dir.join("admission.sock")
    }
    /// Bounded, fresh admission activity of this process (P4 observation).
    pub fn observation(
        &self,
    ) -> Arc<dyn crate::application::admission::ports::inference_observation::AdmissionObservation>
    {
        self.recorder.clone()
    }
    /// Receive a fresh view after every admission transition of this process.
    pub fn on_transition(&self, hook: super::observed_gate::ActivityHook) {
        self.recorder.set_hook(hook);
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
                    "admission authority unreachable: directory {} ({e}); its broker's log is {}/broker.log",
                    directory.display(),
                    directory.display()
                )
            })?;
            let endpoint = dir.client_socket();
            let connection = AuthorityConnection::connect(&endpoint).await.map_err(|e| {
                format!(
                    "admission authority unreachable at {} ({e}); its broker's log is {}/broker.log",
                    endpoint.display(),
                    directory.display()
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
        scope: crate::domain::admission::value_objects::inference_admission::ScopeId {
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
    let (kind, directory) = match &negotiation {
        Negotiation::Root { directory } => (BindingKind::Root, directory.clone()),
        // The client directory is `<authority>/client`; its parent is the
        // authority directory a child is bound to.
        Negotiation::Child { .. } => (
            BindingKind::Child,
            client_dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| client_dir.clone()),
        ),
    };
    let proposal = connection.hello().proposal.clone();
    // Every gate routes through one link so a root can reconnect after a
    // broker restart/reset without the agent restarting (#2024 S3). A child
    // link has no reconnect and fails closed on loss.
    let link = match kind {
        BindingKind::Root => super::link::AuthorityLink::root(
            connection,
            AuthorityDirectory::from_directory(&directory),
            WorkloadClass::Interactive,
            runtime.handle().clone(),
        ),
        BindingKind::Child => super::link::AuthorityLink::child(connection),
    };
    let recorder = Arc::new(super::observed_gate::AdmissionRecorder::new());
    let mut gates: BTreeMap<String, Arc<dyn AttemptAdmission>> = BTreeMap::new();
    for (alias, group) in &proposal.policy.aliases {
        let max_cooldown_ms = proposal.policy.groups[group].max_cooldown_ms;
        let gate: Arc<dyn AttemptAdmission> =
            Arc::new(super::remote_gate::RemoteAdmission::linked(
                link.clone(),
                alias.clone(),
                max_cooldown_ms,
            ));
        gates.insert(
            alias.clone(),
            Arc::new(super::observed_gate::ObservedAdmission::new(
                gate,
                alias,
                group.clone(),
                recorder.clone(),
            )),
        );
    }
    let client = SingleAttemptClient::build(default_client_builder())
        .map_err(|e| format!("admission HTTP client: {e}"))?;
    let context = AdmissionRuntimeContext::new(proposal.clone(), gates, client)?;
    Ok(ProcessAdmission {
        link,
        context: Arc::new(context),
        proposal,
        client_dir,
        recorder,
        runtime,
        kind,
        directory,
        launch: OnceLock::new(),
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
        let connection = self.link.connection();
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

impl ProcessAdmission {
    /// Remember what a root's broker serves (#2615); the first one stays.
    pub fn remember_launch(&self, launch: AuthorityLaunch) {
        debug_assert_eq!(self.kind, BindingKind::Root, "only a root starts a broker");
        let _ = self.launch.set(launch);
    }

    /// Rejoin the authority after its broker was restarted (#2615): wait
    /// (bounded by `wait`) for the old connection to close, then reconnect
    /// through the link exactly as an attempt would. A child cannot rejoin
    /// (its capability was minted by its parent) and a changed policy is
    /// refused as "restart required"; both are answered as errors.
    pub fn rejoin(&self, wait: std::time::Duration) -> Result<(), String> {
        let link = self.link.clone();
        let handle = self.runtime.handle().clone();
        std::thread::spawn(move || {
            handle.block_on(async move {
                let deadline = tokio::time::Instant::now() + wait;
                while link.connection().is_open() && tokio::time::Instant::now() < deadline {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                link.live_inner()
                    .await
                    .map(drop)
                    .map_err(|error| error.to_string())
            })
        })
        .join()
        .unwrap_or_else(|_| Err("admission rejoin thread panicked".into()))
    }
}

/// How long a rejoin waits for the restarted broker's old connection to close.
const REJOIN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// The `AuthoritySession` port over a process binding slot (#2615): the
/// installed binding in production, a test's own slot in its tests.
#[derive(Debug, Clone, Copy)]
pub struct CurrentProcessSession {
    slot: &'static OnceLock<Arc<ProcessAdmission>>,
}

impl CurrentProcessSession {
    /// Over this process's installed binding.
    pub fn new() -> Self {
        Self { slot: &PROCESS }
    }

    /// Over `slot` (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn over(slot: &'static OnceLock<Arc<ProcessAdmission>>) -> Self {
        Self { slot }
    }
}

impl Default for CurrentProcessSession {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthoritySession for CurrentProcessSession {
    fn joined(&self) -> Option<JoinedAuthority> {
        let binding = self.slot.get()?;
        match binding.kind {
            BindingKind::Root => binding.launch.get().cloned().map(JoinedAuthority::Root),
            BindingKind::Child => Some(JoinedAuthority::Child),
        }
    }

    fn rejoin(&self) -> Result<(), RejoinRefusal> {
        let Some(binding) = self.slot.get() else {
            return Err(RejoinRefusal::Failed(
                "admission is not configured for this session".into(),
            ));
        };
        binding
            .rejoin(REJOIN_WAIT)
            .map_err(|reason| match binding.restart_required() {
                true => RejoinRefusal::PolicyChanged,
                false => RejoinRefusal::Failed(reason),
            })
    }

    fn health(&self) -> String {
        self.slot
            .get()
            .map_or("unavailable", |binding| binding.health().as_str())
            .to_string()
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
