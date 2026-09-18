//! Script-managed adapters of the environments capability's effect ports
//! (#1939): the retained `inspect` / `kill` / `cleanup` argv of an
//! environment ([`EnvironmentProcessCommands`]) and the hosted coordination
//! store the supervising session reads by path ([`HostedSwarmRunObservation`],
//! #1924). Mechanics only: whether and when each runs is decided by the
//! application use cases.
use crate::application::environments::ports::{
    EnvironmentProcessCommands, HostedSwarmRunObservation, PortFuture,
};
use crate::domain::environment_registry::EnvironmentRecord;
use crate::domain::environment_retention::{CoordinatorLoss, HostedSwarmRun, SwarmRunObservation};
use crate::infrastructure::processes::containers::script_stderr::{
    ScriptStdout, run_sync_capturing_stderr_tail,
};

/// The retained script argv run against the environment's runtime id.
/// By default every invocation is offloaded to a blocking worker when a
/// runtime is present, so a slow container script never occupies an async
/// thread; [`ScriptEnvironmentCommands::inline`] runs scripts on the calling
/// thread for callers that already sit on a blocking worker (the
/// final-member cleanup jobs).
#[derive(Debug, Clone, Copy)]
pub struct ScriptEnvironmentCommands {
    offload: bool,
}

impl Default for ScriptEnvironmentCommands {
    fn default() -> Self {
        Self { offload: true }
    }
}

impl ScriptEnvironmentCommands {
    /// Scripts run on the calling thread, which must not be an async
    /// runtime worker.
    pub const fn inline() -> Self {
        Self { offload: false }
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
            Some(handle) => match handle.spawn_blocking(job).await {
                Ok(value) => Ok(value),
                Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
                Err(error) => Err(format!("retained script runner stopped: {error}")),
            },
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
        Box::pin(async move {
            self.blocking(move || run_kill_sync(&environment_id, &argv))
                .await?
        })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, ()> {
        let environment_id = environment_id.to_owned();
        let argv = argv.to_vec();
        Box::pin(async move {
            if let Err(error) = self
                .blocking(move || run_script_sync(&environment_id, &argv))
                .await
            {
                tracing::warn!(%error, "retained cleanup could not be run");
            }
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
fn run_script_sync(environment_id: &str, argv: &[String]) {
    if argv.is_empty() {
        return;
    }
    if let Err(error) =
        crate::infrastructure::processes::containers::retained_scripts::run_cleanup_sync(
            environment_id,
            argv,
        )
    {
        tracing::warn!(environment_id, "{error}");
    }
}

/// The retained-kill invocation: argv exec, `QUECTO_CONTAINER_ENVIRONMENT_ID`,
/// stderr kept as a bounded tail. The environment is stopped only on success.
pub(super) fn run_kill_sync(environment_id: &str, argv: &[String]) -> Result<(), String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("no retained kill argv".to_string());
    };
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", environment_id);
    match run_sync_capturing_stderr_tail(cmd, ScriptStdout::Discard, std::time::Duration::MAX) {
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

/// The checkout the host may open a coordination store at (#1924): the
/// create result's `metadata.checkout` when present, else the first of
/// `<workspace>/repo` and `<workspace>` holding a store (older or third-party
/// script sets). Either way the path must be absolute, `..`-free and — after
/// resolving symlinks on both sides — at or under the record's workspace,
/// the only host location a script-managed environment owns. Anything else
/// is treated as no store at all rather than a place to run an interpreter.
pub(super) fn hosted_checkout(record: &EnvironmentRecord) -> Option<std::path::PathBuf> {
    match record
        .metadata
        .get(CHECKOUT_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .filter(|checkout| !checkout.is_empty())
    {
        Some(advertised) => contained_checkout(record, std::path::Path::new(advertised)),
        None => [
            record.workspace_path.join(REPO_CHECKOUT_SUBDIR),
            record.workspace_path.clone(),
        ]
        .iter()
        .filter(|candidate| super::swarm_bridge::store_database(candidate).is_file())
        .find_map(|candidate| contained_checkout(record, candidate)),
    }
}

fn contained_checkout(
    record: &EnvironmentRecord,
    checkout: &std::path::Path,
) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let plain = |path: &std::path::Path| {
        path.is_absolute()
            && path
                .components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
    };
    if !plain(checkout) || !plain(&record.workspace_path) {
        return None;
    }
    // Resolve symlinks on both sides so a link under the workspace pointing
    // elsewhere cannot lead the host outside it.
    let resolved = std::fs::canonicalize(checkout).ok()?;
    let workspace = std::fs::canonicalize(&record.workspace_path).ok()?;
    resolved.starts_with(&workspace).then_some(resolved)
}

fn hosted_store(record: &EnvironmentRecord) -> Option<super::swarm_bridge::HostedStore> {
    hosted_checkout(record).map(super::swarm_bridge::HostedStore::at)
}

/// The coordination store an environment hosts, read by path from the
/// supervising session (#1924).
#[derive(Debug, Default, Clone, Copy)]
pub struct HostedStoreObservation;

impl HostedSwarmRunObservation for HostedStoreObservation {
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, SwarmRunObservation> {
        Box::pin(async move {
            let Some(store) = hosted_store(record) else {
                return SwarmRunObservation::NoStore;
            };
            match store.hosted_run() {
                Ok(Some(run)) => SwarmRunObservation::Run(run),
                Ok(None) => SwarmRunObservation::NoStore,
                Err(error) => {
                    tracing::warn!(
                        environment_id = %record.environment_id,
                        %error,
                        "hosted swarm run could not be observed; environment retained"
                    );
                    SwarmRunObservation::Unreadable(error.to_string())
                }
            }
        })
    }

    fn record_lost_coordinator<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> PortFuture<'a, Result<CoordinatorLoss, String>> {
        Box::pin(async move {
            let store = hosted_store(record)
                .ok_or_else(|| "environment advertises no checkout".to_string())?;
            store
                .record_lost_coordinator(&hosted.coordinator)
                .map_err(|error| error.to_string())
        })
    }
}

#[cfg(test)]
#[path = "environment_commands_tests.rs"]
mod tests;
