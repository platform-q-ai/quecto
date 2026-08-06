//! Exactly-once teardown for registered subagent entries.
//!
//! Slice 2 (#1369): environment children are members of an authoritative
//! session environment record. Tearing a member down removes its membership;
//! the removal that empties a running environment atomically claims the final
//! cleanup and runs the environment's retained `kill` argv exactly once
//! (falling back to the retained `cleanup` argv for script sets without a
//! configured kill). A launch-rollback teardown (initial-prompt failure after
//! registration) runs the retained `cleanup` instead of `kill`, per the
//! documented script contract. Stopped records stay listed; refs are never
//! reused.

use std::sync::Arc;

use super::subagent_registry::SubagentRegistry;
use crate::domain::environment_finalization::{
    EnvironmentFinalizationPort, EnvironmentFinalizationUseCase, MemberFinalizeMode,
};
use crate::domain::environment_registry::EnvironmentRegistry;
use crate::domain::subagent_launch::LaunchFuture;

/// Why a member is being finalized. Normal exits stop the emptied environment
/// with the retained `kill`; a launch rollback (the launch failed after the
/// environment was created) must run the retained `cleanup` instead, per the
/// documented `cleanup` contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinalizeMode {
    Exit,
    /// Parent-initiated `kill_container` termination: death by our own hand
    /// is not a post-mortem, so no inspect runs and no inspect failure can
    /// stick to a cleanly killed environment. The environment-level kill and
    /// status transitions belong to the caller's already-held kill claim.
    ParentKill,
    /// Rollback of a failed join into an environment someone else created.
    LaunchRollback,
    /// Rollback of the launch that created the environment: the environment
    /// never became usable, so after the retained cleanup runs its record is
    /// discarded entirely (matching pre-registration rollback, which leaves
    /// no committed entry) instead of being listed as stopped.
    LaunchRollbackOwned,
}

/// One claimed teardown unit: an optional environment membership to remove
/// (with its finalize mode) and an optional per-entry cleanup plan.
struct CleanupJob {
    membership: Option<(EnvironmentRegistry, String, String)>,
    plan: Option<(String, Vec<String>)>,
    mode: FinalizeMode,
}

pub(super) async fn cleanup_registered_once(registry: &SubagentRegistry, agent_id: &str) {
    let (plan, membership) = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get_mut(agent_id) {
            Some(entry) => (
                entry
                    .cleanup_environment_id
                    .take()
                    .map(|env| (env, std::mem::take(&mut entry.cleanup_argv))),
                take_environment_membership(entry),
            ),
            None => (None, None),
        }
    };
    run_cleanup_jobs(vec![CleanupJob {
        membership: membership.map(|(envs, env_ref)| (envs, env_ref, agent_id.to_string())),
        plan,
        mode: FinalizeMode::Exit,
    }])
    .await;
}

fn take_environment_membership(
    entry: &mut super::subagent_registry::SubagentEntry,
) -> Option<(EnvironmentRegistry, String)> {
    entry
        .environment_registry
        .take()
        .zip(entry.environment_ref.take())
}

fn drain_cleanup_jobs(
    removed: &mut [(String, super::subagent_registry::SubagentEntry)],
    mode: FinalizeMode,
) -> Vec<CleanupJob> {
    removed
        .iter_mut()
        .map(|(id, entry)| CleanupJob {
            plan: entry
                .cleanup_environment_id
                .take()
                .map(|env| (env, std::mem::take(&mut entry.cleanup_argv))),
            membership: take_environment_membership(entry)
                .map(|(envs, env_ref)| (envs, env_ref, id.clone())),
            mode,
        })
        .collect()
}

fn run_cleanup_jobs_sync(jobs: Vec<CleanupJob>) {
    for job in jobs {
        match job.membership {
            Some((environments, env_ref, agent_uuid)) => {
                let use_case = EnvironmentFinalizationUseCase::new(
                    environments,
                    Arc::new(ScriptEnvironmentFinalizationPort),
                );
                futures::executor::block_on(use_case.finalize_member(
                    &env_ref,
                    &agent_uuid,
                    job.plan,
                    job.mode.into(),
                ));
            }
            None => {
                if let Some((env, argv)) = job.plan {
                    run_script_sync(&env, &argv);
                }
            }
        }
    }
}

/// Run claimed teardown jobs on a blocking worker so slow container scripts
/// cannot stall the async runtime. A detached `spawn_blocking` task runs to
/// completion even when the awaiting caller is aborted mid-cleanup, so a
/// claimed plan can never be lost.
async fn run_cleanup_jobs(jobs: Vec<CleanupJob>) {
    let has_work = jobs
        .iter()
        .any(|job| job.membership.is_some() || job.plan.is_some());
    if !has_work {
        return;
    }
    let handle = tokio::task::spawn_blocking(move || run_cleanup_jobs_sync(jobs));
    let _ = handle.await;
}

/// Async teardown for cascade-removed entries; use from async contexts so
/// container scripts run on a blocking worker (never on the runtime thread).
pub(super) async fn cleanup_removed_entries_once(
    removed: &mut [(String, super::subagent_registry::SubagentEntry)],
    mode: FinalizeMode,
) {
    run_cleanup_jobs(drain_cleanup_jobs(removed, mode)).await;
}

/// Synchronous teardown for the process-shutdown path only, where no runtime
/// is guaranteed and blocking the caller is acceptable.
pub(super) fn cleanup_removed_entries_sync(
    removed: &mut [(String, super::subagent_registry::SubagentEntry)],
) {
    // Process shutdown is parent-initiated, not a post-mortem: skip inspect.
    run_cleanup_jobs_sync(drain_cleanup_jobs(removed, FinalizeMode::ParentKill));
}

impl From<FinalizeMode> for MemberFinalizeMode {
    fn from(mode: FinalizeMode) -> Self {
        match mode {
            FinalizeMode::Exit => Self::Exit,
            FinalizeMode::ParentKill => Self::ParentKill,
            FinalizeMode::LaunchRollback => Self::LaunchRollback,
            FinalizeMode::LaunchRollbackOwned => Self::LaunchRollbackOwned,
        }
    }
}

struct ScriptEnvironmentFinalizationPort;

impl EnvironmentFinalizationPort for ScriptEnvironmentFinalizationPort {
    fn run_retained_inspect<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async move { run_inspect_sync(environment_id, argv) })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<(), String>> {
        Box::pin(async move { run_kill_sync(environment_id, argv) })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, ()> {
        Box::pin(async move { run_script_sync(environment_id, argv) })
    }
}

/// Bound on one retained-inspect invocation. A hung inspect script must not
/// stall the death pipeline indefinitely: the exit signal (and the awaits it
/// wakes) fires only after this job finishes, and `classify_dead_socket`'s
/// grace window is sized just above this bound.
pub(super) const INSPECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Inspect script contract: invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID`,
/// prints one JSON object `{"status": "...", "metadata": {...}}` on stdout.
/// The result is parsed through the same strict wire path as create/exec:
/// unknown keys, trailing data, and non-UTF8 output are rejected. The
/// subprocess is bounded by [`INSPECT_TIMEOUT`]; on timeout it is killed and
/// an inspect failure is persisted with the retained argv kept for retry.
fn run_inspect_sync(environment_id: &str, argv: &[String]) -> Result<serde_json::Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InspectResultWire {
        #[serde(default)]
        status: Option<String>,
        metadata: serde_json::Value,
    }
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", environment_id);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let output = output_with_timeout(cmd, INSPECT_TIMEOUT)?;
    if !output.status.success() {
        return Err(format!(
            "retained inspect exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let wire: InspectResultWire =
        super::spawn_container::parse_strict_wire(&output.stdout, "inspect")?;
    if !wire.metadata.is_object() {
        return Err("retained inspect result must contain a metadata object".to_string());
    }
    let mut metadata = wire.metadata;
    if let (Some(object), Some(status)) = (metadata.as_object_mut(), wire.status) {
        object.insert("inspect_status".to_string(), serde_json::json!(status));
    }
    Ok(metadata)
}

/// Run a command to completion with a hard wall-clock bound, killing it on
/// timeout. Runs on a blocking worker (teardown jobs), so the poll loop never
/// occupies an async runtime thread. Output pipes are drained after exit;
/// inspect payloads are one small JSON object, far below pipe capacity.
pub(super) fn output_with_timeout(
    mut cmd: std::process::Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    use std::io::Read;
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to invoke retained inspect: {e}"))?;
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "retained inspect timed out after {}s and was killed; retained argv kept for retry",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => return Err(format!("failed to reap retained inspect: {e}")),
        }
    };
    let mut stdout = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr);
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn run_script_sync(environment_id: &str, argv: &[String]) {
    if argv.is_empty() {
        return;
    }
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", environment_id);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let _ = cmd.status();
}

/// Build the retained-kill invocation. Single definition of the kill script
/// contract (argv exec, `QUECTO_CONTAINER_ENVIRONMENT_ID`, null stdout,
/// piped stderr) shared by the sync final-member path and the async
/// `kill_container` port.
pub(super) fn retained_kill_command(
    environment_id: &str,
    argv: &[String],
) -> std::process::Command {
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", environment_id);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());
    cmd
}

/// Map a retained-kill invocation result to the shared error contract.
pub(super) fn retained_kill_outcome(
    result: std::io::Result<std::process::Output>,
) -> Result<(), String> {
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "retained kill exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(format!("failed to invoke retained kill: {e}")),
    }
}

/// Run the retained kill argv; the environment is stopped only on success.
pub(super) fn run_kill_sync(environment_id: &str, argv: &[String]) -> Result<(), String> {
    if argv.is_empty() {
        return Err("no retained kill argv".to_string());
    }
    retained_kill_outcome(retained_kill_command(environment_id, argv).output())
}
