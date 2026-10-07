//! Script-managed adapters of the environments capability's effect ports
//! (#1939): the retained `inspect` / `kill` / `cleanup` argv of an
//! environment ([`EnvironmentProcessCommands`]) and the hosted coordination
//! store the supervising session reads by path ([`HostedSwarmRunObservation`],
//! #1924). Mechanics only: whether and when each runs is decided by the
//! application use cases.
use crate::application::environments::dto::EnvironmentLiveness;
use crate::application::environments::ports::EnvironmentProcess;
use crate::application::environments::ports::{
    EnvironmentProcessCommands, HostedSwarmRunInspection, HostedSwarmRunObservation, PortFuture,
};
use crate::domain::environments::entities::environment_registry::EnvironmentRecord;
use crate::domain::environments::services::environment_retention::{
    CoordinatorLoss, HostedSwarmRun, SwarmRunObservation,
};
use crate::infrastructure::processes::containers::environment_process::ScriptEnvironmentProcess;
use crate::infrastructure::processes::containers::script_stderr::{
    ScriptStdout, run_sync_capturing_stderr_tail,
};
use crate::infrastructure::processes::containers::standard::integrity::refuse_altered_script;
use crate::infrastructure::tools::swarm_bridge::SwarmBoard;

/// The retained script argv run against the environment's runtime id.
/// By default every invocation is offloaded to a blocking worker when a
/// runtime is present, so a slow container script never occupies an async
/// thread; [`ScriptEnvironmentCommands::inline`] runs scripts on the calling
/// thread for callers that already sit on a blocking worker (the
/// final-member cleanup jobs).
#[derive(Debug, Clone, Copy)]
pub struct ScriptEnvironmentCommands {
    offload: bool,
    /// Bound on one retained kill script (#2070): past it the script is
    /// killed and the kill reported failed, so the record is `cleanup-failed`
    /// with the reason and retryable — never a harness parked behind a
    /// runtime that hangs on its remove. A kill takes seconds; an owner's
    /// exit waits on this at most once.
    kill_bound: std::time::Duration,
    /// Bound on one retained cleanup script (#2206), past which it is
    /// killed and reported failed like a hung kill.
    cleanup_bound: std::time::Duration,
}

/// The default bound on one retained kill script.
pub const KILL_SCRIPT_BOUND: std::time::Duration = std::time::Duration::from_secs(20);

/// The default bound on one retained cleanup script (#2206). Generous and
/// distinct from the kill's: a cleanup removes the whole state directory —
/// a full repository clone, its build output — and a large `rm -rf` on a
/// slow disk can take far longer than stopping a container, while a
/// cleanup cut short leaves exactly the leftovers it was meant to remove.
/// Nothing on an exit budget waits on it: launch rollbacks are waited for
/// under their own limit, and the owner's end of a plain child runs it on
/// a blocking worker. It only guards against a runtime that never returns.
pub const CLEANUP_SCRIPT_BOUND: std::time::Duration = std::time::Duration::from_secs(120);

impl Default for ScriptEnvironmentCommands {
    fn default() -> Self {
        Self {
            offload: true,
            kill_bound: KILL_SCRIPT_BOUND,
            cleanup_bound: CLEANUP_SCRIPT_BOUND,
        }
    }
}

impl ScriptEnvironmentCommands {
    /// Scripts run on the calling thread, which must not be an async
    /// runtime worker.
    pub const fn inline() -> Self {
        Self {
            offload: false,
            kill_bound: KILL_SCRIPT_BOUND,
            cleanup_bound: CLEANUP_SCRIPT_BOUND,
        }
    }

    #[cfg(test)]
    pub fn with_kill_bound(mut self, kill_bound: std::time::Duration) -> Self {
        self.kill_bound = kill_bound;
        self
    }

    #[cfg(test)]
    pub fn with_cleanup_bound(mut self, cleanup_bound: std::time::Duration) -> Self {
        self.cleanup_bound = cleanup_bound;
        self
    }

    /// Run `job` off the async runtime when one is present and offloading
    /// is wanted; a detached `spawn_blocking` task runs to completion even
    /// when the awaiting caller is aborted mid-script, so a claimed script
    /// can never be lost. A runtime that shuts down before the job ran is
    /// reported, never mistaken for a script that ran.
    async fn blocking<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, String> {
        let handle = if self.offload {
            tokio::runtime::Handle::try_current().ok()
        } else {
            None
        };
        match handle {
            // A panic in the script job resumes here, in the call.
            Some(handle) => super::call_work::spawn_blocking_in_call_on(&handle, job)
                .await
                .map_err(|error| format!("retained script runner stopped: {error}")),
            None => Ok(job()),
        }
    }
}

impl EnvironmentProcessCommands for ScriptEnvironmentCommands {
    fn run_retained_inspect<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>> {
        let environment_id = environment_id.to_owned();
        let argv = argv.to_vec();
        Box::pin(async move {
            self.blocking(move || run_inspect_sync(&environment_id, &argv))
                .await?
        })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        let environment_id = environment_id.to_owned();
        let argv = argv.to_vec();
        let bound = self.kill_bound;
        Box::pin(async move {
            self.blocking(move || run_kill_sync(&environment_id, &argv, bound))
                .await?
        })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        let environment_id = environment_id.to_owned();
        let argv = argv.to_vec();
        let bound = self.cleanup_bound;
        Box::pin(async move {
            self.blocking(move || run_cleanup_script_sync(&environment_id, &argv, bound))
                .await
                .inspect_err(|error| tracing::warn!(%error, "retained cleanup could not be run"))?
        })
    }

    fn observe_liveness<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, EnvironmentLiveness> {
        let record = record.clone();
        Box::pin(async move {
            self.blocking(move || ScriptEnvironmentProcess.observe(&record))
                .await
                .unwrap_or_else(EnvironmentLiveness::Unknown)
        })
    }
}

pub(super) use crate::infrastructure::processes::containers::retained_scripts::INSPECT_TIMEOUT;

/// Inspect script contract: see `retained_scripts::run_inspect_sync_bounded`;
/// bounded by [`INSPECT_TIMEOUT`] here.
fn run_inspect_sync(environment_id: &str, argv: &[String]) -> Result<serde_json::Value, String> {
    run_inspect_sync_bounded(environment_id, argv, INSPECT_TIMEOUT)
}

pub(super) use crate::infrastructure::processes::containers::retained_scripts::run_inspect_sync_bounded;

/// The retained-cleanup invocation: best effort by contract, never silent
/// (#2024 S4b) — a cleanup that fails leaves an environment behind and
/// its stderr tail is the operator's only lead.
fn run_cleanup_script_sync(
    environment_id: &str,
    argv: &[String],
    bound: std::time::Duration,
) -> Result<(), String> {
    crate::infrastructure::processes::containers::retained_scripts::run_cleanup_sync_bounded(
        environment_id,
        argv,
        bound,
    )
    .inspect_err(|error| tracing::warn!(environment_id, "{error}"))
}

/// The retained-kill invocation: argv exec, `QUECTO_CONTAINER_ENVIRONMENT_ID`,
/// stderr kept as a bounded tail. The environment is stopped only on success.
/// An altered standard script is refused before it runs (the use case
/// leaves the retryable `cleanup-failed` state with the reason).
pub(super) fn run_kill_sync(
    environment_id: &str,
    argv: &[String],
    bound: std::time::Duration,
) -> Result<(), String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("no retained kill argv".to_string());
    };
    refuse_altered_script(argv).map_err(|reason| format!("retained kill refused: {reason}"))?;
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", environment_id);
    match run_sync_capturing_stderr_tail(cmd, ScriptStdout::Discard, bound) {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "retained kill exited with {}: {}",
            output.status, output.stderr_tail
        )),
        Err(error) => Err(format!("failed to invoke retained kill: {error}")),
    }
}

// ─── Hosted swarm run observation (#1924) ────────────────────────────────────

/// Create-result metadata key naming the members' checkout: the directory the
/// container adapter hands members as their working directory and swarm
/// checkout root, identity-mounted so the supervising session reads the
/// coordination store there (#1924). Absent for script sets that host no
/// swarm; the ordinary final-member teardown then applies.
pub(super) const CHECKOUT_METADATA_KEY: &str = "checkout";

/// Sub-directory a repository-cloning script set checks the source out
/// into, probed when a create result predates `metadata.checkout`.
const REPO_CHECKOUT_SUBDIR: &str = "repo";

/// Where the host may open a coordination store for a record (#1924,
/// #2206 rounds 2 and 3).
///
/// The checkout is the create result's `metadata.checkout` when present,
/// else the first of `<workspace>/repo` and `<workspace>` where a board
/// may be (older or third-party script sets). Either way the path must be
/// absolute, `..`-free and — after resolving symlinks on both sides — at
/// or under the record's workspace, the only host location a
/// script-managed environment owns; a checkout that does not resolve
/// there (renamed, unreadable, a link loop, outside) is `Unknown`, never a
/// place to run an interpreter and never "no board".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HostedCheckout {
    /// The checkout to read, at or under the record's workspace.
    At(std::path::PathBuf),
    /// Proven to hold no board: the workspace is on this host and every
    /// board place answered "no such file" — the only answer that lets a
    /// box count as a plain container.
    NoBoard,
    /// No advertised checkout and the workspace is absent from this host
    /// (a script set whose workspace lives only in the container): nothing
    /// the host could read, and nothing proven either.
    NotOnHost,
    /// The host could not tell; the environment is kept.
    Unknown(String),
}

pub(super) fn hosted_checkout(record: &EnvironmentRecord) -> HostedCheckout {
    match record
        .metadata
        .get(CHECKOUT_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .filter(|checkout| !checkout.is_empty())
    {
        Some(advertised) => {
            match contained_below(&record.workspace_path, std::path::Path::new(advertised)) {
                Ok(checkout) => HostedCheckout::At(checkout),
                Err(reason) => HostedCheckout::Unknown(reason),
            }
        }
        None => first_with_board(
            &record.workspace_path,
            [
                record.workspace_path.join(REPO_CHECKOUT_SUBDIR),
                record.workspace_path.clone(),
            ],
        ),
    }
}

/// The first candidate checkout that may hold a board, contained below
/// `root`. With every candidate's every board place answering "no such
/// file": `NoBoard` when `root` is on this host, `NotOnHost` when it is
/// not.
fn first_with_board(root: &std::path::Path, candidates: [std::path::PathBuf; 2]) -> HostedCheckout {
    use super::swarm_store_location::BoardPresence;
    for candidate in candidates {
        match super::swarm_store_location::board_presence(&candidate) {
            BoardPresence::Absent => {}
            BoardPresence::Current { .. } | BoardPresence::Displaced(_) => {
                return match contained_below(root, &candidate) {
                    Ok(checkout) => HostedCheckout::At(checkout),
                    Err(reason) => HostedCheckout::Unknown(reason),
                };
            }
            BoardPresence::Unknown(reason) => return HostedCheckout::Unknown(reason),
        }
    }
    match std::fs::symlink_metadata(root) {
        Ok(_) => HostedCheckout::NoBoard,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HostedCheckout::NotOnHost,
        Err(error) => HostedCheckout::Unknown(format!("workspace {}: {error}", root.display())),
    }
}

/// `checkout` resolved, when both it and `root` are absolute and `..`-free
/// and — symlinks resolved on both sides, so a link under the root pointing
/// elsewhere cannot lead the host outside it — it lies at or under `root`;
/// otherwise why not.
fn contained_below(
    root: &std::path::Path,
    checkout: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    use std::path::Component;
    let plain = |path: &std::path::Path| {
        path.is_absolute()
            && path
                .components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
    };
    match plain(checkout) && plain(root) {
        true => {}
        false => {
            return Err(format!(
                "checkout {} is not a plain absolute path below {}",
                checkout.display(),
                root.display()
            ));
        }
    }
    let resolved = std::fs::canonicalize(checkout)
        .map_err(|error| format!("checkout {}: {error}", checkout.display()))?;
    let root = std::fs::canonicalize(root)
        .map_err(|error| format!("workspace {}: {error}", root.display()))?;
    match resolved.starts_with(&root) {
        true => Ok(resolved),
        false => Err(format!(
            "checkout {} resolves outside its workspace {}",
            resolved.display(),
            root.display()
        )),
    }
}

fn hosted_store(
    record: &EnvironmentRecord,
    board: &SwarmBoard,
) -> Option<super::swarm_bridge::HostedStore> {
    match hosted_checkout(record) {
        HostedCheckout::At(checkout) => Some(super::swarm_bridge::HostedStore::at(
            checkout,
            board.clone(),
        )),
        HostedCheckout::NoBoard | HostedCheckout::NotOnHost | HostedCheckout::Unknown(_) => None,
    }
}

/// The checkout a store may live at below a bare environment state
/// directory the shipped scripts laid out (`<state_dir>/workspace[/repo]`),
/// for a directory no record names: the first of the two where a board may
/// be, contained below the state dir the same way a record's checkout is
/// contained below its workspace. A directory the collector found but
/// whose workspace was never made has no board.
fn unrecorded_checkout(state_dir: &std::path::Path) -> HostedCheckout {
    let workspace = state_dir.join("workspace");
    match first_with_board(state_dir, [workspace.join(REPO_CHECKOUT_SUBDIR), workspace]) {
        // The collector judges directories on this host: one that is not
        // (or no longer) there holds nothing, board included.
        HostedCheckout::NotOnHost => HostedCheckout::NoBoard,
        judged @ (HostedCheckout::At(_) | HostedCheckout::NoBoard | HostedCheckout::Unknown(_)) => {
            judged
        }
    }
}

/// One synchronous read of the store at `checkout`, for the environment
/// named `subject` in the log.
fn read_hosted_run(
    checkout: HostedCheckout,
    subject: &str,
    board: &SwarmBoard,
) -> SwarmRunObservation {
    let store = match checkout {
        HostedCheckout::At(checkout) => {
            super::swarm_bridge::HostedStore::at(checkout, board.clone())
        }
        HostedCheckout::NoBoard => return SwarmRunObservation::NoStore,
        HostedCheckout::NotOnHost => return SwarmRunObservation::NoStoreUnverified,
        HostedCheckout::Unknown(error) => {
            tracing::warn!(
                environment = %subject,
                %error,
                "hosted swarm run's checkout could not be judged; environment retained"
            );
            return SwarmRunObservation::Unreadable(error);
        }
    };
    match store.hosted_run() {
        Ok(Some(run)) => SwarmRunObservation::Run(run),
        Ok(None) => SwarmRunObservation::NoStore,
        Err(error) => {
            tracing::warn!(
                environment = %subject,
                %error,
                "hosted swarm run could not be observed; environment retained"
            );
            SwarmRunObservation::Unreadable(error.to_string())
        }
    }
}

/// The coordination store an environment hosts, read by path from the
/// supervising session (#1924) — asynchronously for the finalizer, and
/// synchronously for the restore and the collector (round 4 M1, #2033).
/// The store is reached through composition's board handles (#2278).
#[derive(Debug, Clone)]
pub struct HostedStoreObservation {
    board: SwarmBoard,
}

impl HostedStoreObservation {
    /// The observation over `board`: composition's, the process's board
    /// when admission bound one (#2278 review M1).
    pub fn new(board: SwarmBoard) -> Self {
        Self { board }
    }
}

impl HostedSwarmRunObservation for HostedStoreObservation {
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, SwarmRunObservation> {
        let (observation, record) = (self.clone(), record.clone());
        Box::pin(async move {
            off_the_workers(move || observation.inspect_hosted_run(&record))
                .await
                .unwrap_or_else(SwarmRunObservation::Unreadable)
        })
    }

    fn record_lost_coordinator<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> PortFuture<'a, Result<CoordinatorLoss, String>> {
        let store = hosted_store(record, &self.board);
        let coordinator = hosted.coordinator.clone();
        Box::pin(async move {
            let store = store.ok_or_else(|| "environment advertises no checkout".to_string())?;
            off_the_workers(move || {
                store
                    .record_lost_coordinator(&coordinator)
                    .map_err(|error| error.to_string())
            })
            .await?
        })
    }
}

/// Runs `job`, whose board calls block, off the async workers (#2278
/// review L6); a job cancelled with its runtime answers why.
async fn off_the_workers<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    super::call_work::off_the_workers(job)
        .await
        .map_err(|error| format!("hosted swarm run read did not finish: {error}"))
}

impl HostedSwarmRunInspection for HostedStoreObservation {
    fn inspect_hosted_run(&self, record: &EnvironmentRecord) -> SwarmRunObservation {
        read_hosted_run(hosted_checkout(record), &record.environment_id, &self.board)
    }

    fn inspect_hosted_run_at(&self, state_dir: &std::path::Path) -> SwarmRunObservation {
        read_hosted_run(
            unrecorded_checkout(state_dir),
            &state_dir.display().to_string(),
            &self.board,
        )
    }
}

#[cfg(test)]
#[path = "environment_commands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "environment_commands_board_tests.rs"]
mod board_tests;
