//! Exactly-once teardown for registered subagent entries.
//!
//! Slice 2 (#1369): environment children are members of an authoritative
//! session environment record. Tearing a member down removes its membership;
//! the removal that empties a running environment atomically claims the final
//! cleanup and runs the environment's retained `kill` argv exactly once
//! (falling back to the rollback `cleanup` plan for script sets without a
//! configured kill). Stopped records stay listed; refs are never reused.

use super::subagent_registry::SubagentRegistry;
use crate::domain::environment_registry::EnvironmentRegistry;

pub(super) async fn cleanup_registered_once(registry: &SubagentRegistry, agent_id: &str) {
    let (cleanup_plan, membership) = {
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
    if let Some((environments, env_ref)) = membership {
        // Detached blocking task: a caller aborted mid-cleanup (uncommit
        // aborts the monitor task) must not be able to lose the claimed plan.
        let agent_uuid = agent_id.to_string();
        let handle = tokio::task::spawn_blocking(move || {
            remove_member_and_finalize(&environments, &env_ref, agent_uuid, cleanup_plan);
        });
        let _ = handle.await;
        return;
    }
    if let Some((env, mut argv)) = cleanup_plan {
        let cleanup = tokio::spawn(async move {
            super::spawn_container::run_cleanup_once(Some(env), &mut argv).await;
        });
        let _ = cleanup.await;
    }
}

fn take_environment_membership(
    entry: &mut super::subagent_registry::SubagentEntry,
) -> Option<(EnvironmentRegistry, String)> {
    entry
        .environment_registry
        .take()
        .zip(entry.environment_ref.take())
}

/// Remove one member from its environment; the final removal of a running
/// environment claims and runs the retained kill exactly once.
fn remove_member_and_finalize(
    environments: &EnvironmentRegistry,
    env_ref: &str,
    agent_uuid: String,
    fallback_cleanup_plan: Option<(String, Vec<String>)>,
) {
    let Ok(removal) = environments.remove_member(env_ref, &agent_uuid) else {
        return;
    };
    let Some(claim) = removal.claim else {
        return;
    };
    let Some(record) = environments.get(env_ref) else {
        environments.complete_kill(claim);
        return;
    };
    if record.retained_kill_argv.is_empty() {
        // Script sets without a configured kill fall back to the rollback
        // cleanup plan (slice-1 compatibility); best-effort, then stop.
        if let Some((env_id, argv)) = fallback_cleanup_plan {
            run_script_sync(&env_id, &argv);
        }
        environments.complete_kill(claim);
        return;
    }
    match run_kill_sync(&record.environment_id, &record.retained_kill_argv) {
        Ok(()) => environments.complete_kill(claim),
        Err(e) => environments.fail_kill(claim, &e),
    }
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

/// Run the retained kill argv; the environment is stopped only on success.
pub(super) fn run_kill_sync(environment_id: &str, argv: &[String]) -> Result<(), String> {
    if argv.is_empty() {
        return Err("no retained kill argv".to_string());
    }
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", environment_id);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());
    match cmd.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "retained kill exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(e) => Err(format!("failed to invoke retained kill: {e}")),
    }
}

pub(super) fn cleanup_removed_entries_once(
    removed: &mut [(String, super::subagent_registry::SubagentEntry)],
) {
    for (id, entry) in removed.iter_mut() {
        let cleanup_plan = entry
            .cleanup_environment_id
            .take()
            .map(|env| (env, std::mem::take(&mut entry.cleanup_argv)));
        if let Some((environments, env_ref)) = take_environment_membership(entry) {
            remove_member_and_finalize(&environments, &env_ref, id.clone(), cleanup_plan);
        } else if let Some((env, argv)) = cleanup_plan {
            run_script_sync(&env, &argv);
        }
    }
}
