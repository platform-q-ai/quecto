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

use super::subagent_registry::SubagentRegistry;
use crate::domain::environment_registry::EnvironmentRegistry;

/// Why a member is being finalized. Normal exits stop the emptied environment
/// with the retained `kill`; a launch rollback (the launch failed after the
/// environment was created) must run the retained `cleanup` instead, per the
/// documented `cleanup` contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinalizeMode {
    Exit,
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
                remove_member_and_finalize(&environments, &env_ref, agent_uuid, job.plan, job.mode);
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
    run_cleanup_jobs_sync(drain_cleanup_jobs(removed, FinalizeMode::Exit));
}

/// Remove one member from its environment; the final removal of a running
/// environment claims and runs the retained teardown exactly once.
///
/// This transaction deliberately lives next to `remove_member` rather than in
/// `EnvironmentControlUseCase::kill_container`: the final-member claim must be
/// granted atomically with the membership removal (only `begin_kill` and
/// `remove_member` mint claims), so the exit path cannot be routed through the
/// use case's resolve-then-claim sequence without a race window.
fn remove_member_and_finalize(
    environments: &EnvironmentRegistry,
    env_ref: &str,
    agent_uuid: String,
    entry_cleanup_plan: Option<(String, Vec<String>)>,
    mode: FinalizeMode,
) {
    // #1369 slice 3: a normal member death runs the environment's retained
    // inspect exactly once post-mortem, updating the authoritative aggregate
    // BEFORE the member is removed (so the outcome also survives the removal
    // emptying the environment). Launch rollbacks are not deaths: the child
    // never became usable, so no inspect runs.
    if mode == FinalizeMode::Exit {
        run_inspect_once(environments, env_ref, &agent_uuid);
    }
    let Ok(removal) = environments.remove_member(env_ref, &agent_uuid) else {
        return;
    };
    let Some(claim) = removal else {
        return;
    };
    let Some(record) = environments.get(env_ref) else {
        environments.complete_kill(claim);
        return;
    };
    // A launch rollback runs the retained cleanup, never the kill: the
    // documented `cleanup` contract covers launches that fail after creation.
    let launch_rollback = matches!(
        mode,
        FinalizeMode::LaunchRollback | FinalizeMode::LaunchRollbackOwned
    );
    let run_cleanup_instead_of_kill = launch_rollback || record.retained_kill_argv.is_empty();
    if run_cleanup_instead_of_kill {
        // Prefer the cleanup argv retained on the environment record — it
        // survives the creator exiting first — falling back to the removed
        // entry's own claimed plan (slice-1 compatibility). Best-effort, then
        // stop.
        if !record.retained_cleanup_argv.is_empty() {
            run_script_sync(&record.environment_id, &record.retained_cleanup_argv);
        } else if let Some((env_id, argv)) = entry_cleanup_plan {
            run_script_sync(&env_id, &argv);
        }
        environments.complete_kill(claim);
        if mode == FinalizeMode::LaunchRollbackOwned {
            environments.remove(env_ref);
        }
        return;
    }
    match run_kill_sync(&record.environment_id, &record.retained_kill_argv) {
        Ok(()) => environments.complete_kill(claim),
        Err(e) => environments.fail_kill(claim, &e),
    }
}

/// Run the environment's retained inspect for one dead member, exactly once:
/// only the death signal that wins the `begin_inspect` claim invokes the
/// script; repeated EOF/reset for the same member claims nothing. Success
/// merges the inspect metadata into the aggregate; failure persists an
/// actionable error while keeping the retained argv for retry.
fn run_inspect_once(environments: &EnvironmentRegistry, env_ref: &str, agent_uuid: &str) {
    let Some(record) = environments.get(env_ref) else {
        return;
    };
    if record.retained_inspect_argv.is_empty() {
        return;
    }
    let Some(claim) = environments.begin_inspect(env_ref, agent_uuid) else {
        return;
    };
    match run_inspect_sync(&record.environment_id, &record.retained_inspect_argv) {
        Ok(metadata) => environments.record_inspect_success(claim, metadata),
        Err(e) => environments.record_inspect_failure(claim, &e),
    }
}

/// Inspect script contract: invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID`,
/// prints one JSON object `{"status": "...", "metadata": {...}}` on stdout.
fn run_inspect_sync(environment_id: &str, argv: &[String]) -> Result<serde_json::Value, String> {
    #[derive(serde::Deserialize)]
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
    let output = cmd
        .output()
        .map_err(|e| format!("failed to invoke retained inspect: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "retained inspect exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let wire: InspectResultWire = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("retained inspect returned invalid JSON contract: {e}"))?;
    if !wire.metadata.is_object() {
        return Err("retained inspect result must contain a metadata object".to_string());
    }
    let mut metadata = wire.metadata;
    if let (Some(object), Some(status)) = (metadata.as_object_mut(), wire.status) {
        object.insert("inspect_status".to_string(), serde_json::json!(status));
    }
    Ok(metadata)
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
