//! Authority process: singleton lock, owner token, durable journal and two
//! framed UDS listeners (client and owner-only admin). Session frames are
//! decoded in `session`; the one serialized state machine they drive is
//! `server_actor`.
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::net::UnixListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::directory::{AuthorityDirectory, SingletonLock};
use super::protocol::*;
use super::secret::RandomSecretSource;
use super::server_actor::Actor;
use super::session;
use crate::application::admission::dto::broker::{AuthorityStart, RestoreError};
use crate::application::admission::use_cases::{
    AdvanceAuthorityEpoch, BindScope, CompleteAdmission, DisconnectScope, RestoreAuthority,
    SettleAuthority,
};
use crate::infrastructure::providers::routing::provider_runtime_admission::AdmissionRuntimeProposal;

pub(super) use super::server_actor::Command;

/// The running authority's use cases (#1910), built by composition over the
/// one journal of the directory this server serves: the durability
/// sequences and the session policy. The actor invokes them and never
/// constructs one.
pub struct AuthorityUseCases {
    pub restore: RestoreAuthority,
    pub settle: SettleAuthority,
    pub complete: CompleteAdmission,
    pub advance_epoch: AdvanceAuthorityEpoch,
    pub bind: BindScope,
    pub disconnect: DisconnectScope,
}

/// Composition's builder of [`AuthorityUseCases`] over a directory's journal.
/// The server calls it for the directory it serves, once it holds that
/// directory's singleton lock, so the use cases can never write another
/// broker's journal.
pub type AuthorityUseCaseBuilder = fn(&AuthorityDirectory) -> AuthorityUseCases;

#[derive(Debug)]
pub enum ServerError {
    Busy(String),
    Io(String),
    Journal(String),
    Policy(String),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(m) | Self::Io(m) | Self::Journal(m) | Self::Policy(m) => f.write_str(m),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Clock {
    start: Instant,
}

impl Clock {
    /// A clock that starts now, for the actor's own tests.
    #[cfg(test)]
    pub(super) fn starting_now() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub(super) fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    pub(super) fn instant_at(&self, ms: u64) -> Instant {
        self.start + Duration::from_millis(ms)
    }
}

pub(super) fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Fresh owner token per start, 0600 in the authority root (never in `client/`).
fn write_owner_token(dir: &AuthorityDirectory) -> Result<String, ServerError> {
    use crate::application::admission::ports::inference_authority_ports::AdmissionSecretSource;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let token = RandomSecretSource.mint();
    let path = dir.root_token_path();
    let _ = std::fs::remove_file(&path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| ServerError::Io(format!("owner token {}: {e}", path.display())))?;
    file.write_all(token.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| ServerError::Io(format!("owner token {}: {e}", path.display())))?;
    Ok(token)
}

fn bind_private(path: &Path) -> Result<UnixListener, ServerError> {
    // The singleton lock is held: any file at this path is a stale socket.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)
        .map_err(|e| ServerError::Io(format!("bind {}: {e}", path.display())))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
        let _ = std::fs::remove_file(path);
        ServerError::Io(format!("chmod {}: {e}", path.display()))
    })?;
    Ok(listener)
}

/// A running authority. Dropping without `shutdown` aborts its tasks.
/// How long a shutdown waits for the actor's final ledger checkpoint.
const SHUTDOWN_CHECKPOINT: Duration = Duration::from_secs(5);

pub struct AuthorityServer {
    dir: AuthorityDirectory,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    _lock: SingletonLock,
}

// The start functions are crate-private (#1910): a broker is started only
// through composition (its test-support start functions) or the
// `admission-broker run` path, never directly from outside the crate.
impl AuthorityServer {
    pub(crate) async fn start(
        dir: AuthorityDirectory,
        proposal: AdmissionRuntimeProposal,
        build_use_cases: AuthorityUseCaseBuilder,
    ) -> Result<Self, ServerError> {
        Self::start_with(dir, proposal, build_use_cases, false, FRAME_DEADLINE).await
    }

    /// Start with a framing deadline other than [`FRAME_DEADLINE`]: the
    /// stalled-frame tests' entry point, so they need not wait out 15 s.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn start_with_frame_deadline(
        dir: AuthorityDirectory,
        proposal: AdmissionRuntimeProposal,
        build_use_cases: AuthorityUseCaseBuilder,
        frame_deadline: Duration,
    ) -> Result<Self, ServerError> {
        Self::start_with(dir, proposal, build_use_cases, false, frame_deadline).await
    }

    /// Operator acknowledgement that a ledger missing after prior operation
    /// may be replaced by an empty one (outstanding remote work is no longer
    /// claimed bounded).
    pub(crate) async fn start_accepting_missing_ledger(
        dir: AuthorityDirectory,
        proposal: AdmissionRuntimeProposal,
        build_use_cases: AuthorityUseCaseBuilder,
    ) -> Result<Self, ServerError> {
        Self::start_with(dir, proposal, build_use_cases, true, FRAME_DEADLINE).await
    }

    async fn start_with(
        dir: AuthorityDirectory,
        proposal: AdmissionRuntimeProposal,
        build_use_cases: AuthorityUseCaseBuilder,
        accept_missing_ledger: bool,
        frame_deadline: Duration,
    ) -> Result<Self, ServerError> {
        proposal
            .policy
            .validate()
            .map_err(|e| ServerError::Policy(format!("invalid admission policy: {e:?}")))?;
        let proposal_wire = ProposalWire::from(&proposal);
        let hello_bytes = serde_json::to_vec(&Reply {
            id: Some(0),
            body: Body::Hello {
                version: PROTOCOL_VERSION,
                role: Role::Client,
                epoch: u64::MAX,
                proposal: proposal_wire.clone(),
            },
        })
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
        if hello_bytes > FRAME_CAP {
            return Err(ServerError::Policy(format!(
                "admission policy is too large to publish ({hello_bytes} bytes > {FRAME_CAP})"
            )));
        }
        // The lock file's existence before this start is the evidence of prior
        // operation: a fresh directory has none.
        let previously_operated = dir.lock_path().exists();
        let lock = SingletonLock::acquire(&dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::WouldBlock => ServerError::Busy(e.to_string()),
            _ => ServerError::Io(e.to_string()),
        })?;
        let clock = Clock {
            start: Instant::now(),
        };
        let use_cases = build_use_cases(&dir);
        let start = AuthorityStart {
            policy: proposal.policy.clone(),
            previously_operated,
            accept_missing_ledger,
        };
        let authority = use_cases
            .restore
            .execute(start, RandomSecretSource, clock.now_ms())
            .map_err(|error| restore_refusal(&dir, error))?;
        let owner_token = write_owner_token(&dir)?;
        let client_listener = bind_private(&dir.client_socket())?;
        let admin_listener = bind_private(&dir.admin_socket())?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let actor = Actor::new(authority, use_cases, clock, proposal_wire, owner_token);
        let tasks = vec![
            tokio::spawn(actor.run(rx, shutdown_rx.clone())),
            tokio::spawn(accept_loop(
                client_listener,
                Role::Client,
                tx.clone(),
                shutdown_rx.clone(),
                1,
                frame_deadline,
            )),
            tokio::spawn(accept_loop(
                admin_listener,
                Role::Admin,
                tx,
                shutdown_rx,
                1 << 62,
                frame_deadline,
            )),
        ];
        Ok(Self {
            dir,
            shutdown,
            tasks,
            _lock: lock,
        })
    }

    pub fn directory(&self) -> &AuthorityDirectory {
        &self.dir
    }

    /// Stop accepting, close sessions and exit the actor. The journal keeps
    /// outstanding work for the next start; sockets are removed.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        let mut tasks = std::mem::take(&mut self.tasks).into_iter();
        // The actor (spawned first) checkpoints its ledger as it stops; let
        // it finish that write, within a bound, before anything is aborted.
        if let Some(mut actor) = tasks.next() {
            if tokio::time::timeout(SHUTDOWN_CHECKPOINT, &mut actor)
                .await
                .is_err()
            {
                actor.abort();
                let _ = actor.await;
            }
        }
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        let _ = std::fs::remove_file(self.dir.client_socket());
        let _ = std::fs::remove_file(self.dir.admin_socket());
    }
}

impl std::fmt::Debug for AuthorityServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityServer")
            .field("directory", &self.dir.path())
            .finish_non_exhaustive()
    }
}

impl Drop for AuthorityServer {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn accept_loop(
    listener: UnixListener,
    role: Role,
    commands: mpsc::UnboundedSender<Command>,
    mut shutdown: watch::Receiver<bool>,
    first_session: u64,
    frame_deadline: Duration,
) {
    let mut next = first_session;
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let session = next;
                    next += 1;
                    tokio::spawn(session::serve(
                        stream,
                        session,
                        role,
                        commands.clone(),
                        frame_deadline,
                        shutdown.clone(),
                    ));
                }
                Err(error) => {
                    tracing::warn!(%error, "admission authority accept failed");
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    }
}

/// The operator-facing refusal for an authority that could not start from
/// its journal; the directory names the ledger the operator must restore.
fn restore_refusal(dir: &AuthorityDirectory, error: RestoreError) -> ServerError {
    ServerError::Journal(match error {
        RestoreError::Unreadable(reason) => reason,
        RestoreError::MissingLedger => format!(
            "ledger {} is missing after prior operation; outstanding remote work cannot be accounted. Restore it or run with --accept-missing-ledger to start an empty ledger",
            dir.journal_path().display()
        ),
        RestoreError::Incompatible(error) => format!("ledger incompatible with policy: {error:?}"),
        RestoreError::NotDurable(error) => format!("initial ledger not durable: {error:?}"),
    })
}
