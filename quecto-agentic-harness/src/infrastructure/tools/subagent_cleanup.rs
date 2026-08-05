use super::subagent_registry::SubagentRegistry;
use crate::domain::environment_registry::EnvironmentRegistry;

pub(super) async fn cleanup_registered_once(registry: &SubagentRegistry, agent_id: &str) {
    let (cleanup_plan, uncommit) = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get_mut(agent_id) {
            Some(entry) => (
                entry
                    .cleanup_environment_id
                    .take()
                    .map(|env| (env, std::mem::take(&mut entry.cleanup_argv))),
                take_environment_uncommit(entry),
            ),
            None => (None, None),
        }
    };
    uncommit_environment(uncommit);
    if let Some((env, mut argv)) = cleanup_plan {
        // No await between the claim above and this spawn: a caller that gets
        // aborted mid-cleanup (uncommit aborts the monitor task) must not be
        // able to lose the claimed plan, so the command runs detached.
        let cleanup = tokio::spawn(async move {
            super::spawn_container::run_cleanup_once(Some(env), &mut argv).await;
        });
        let _ = cleanup.await;
    }
}

fn take_environment_uncommit(
    entry: &mut super::subagent_registry::SubagentEntry,
) -> Option<(EnvironmentRegistry, String)> {
    entry
        .environment_registry
        .take()
        .zip(entry.environment_ref.take())
}

fn uncommit_environment(uncommit: Option<(EnvironmentRegistry, String)>) {
    if let Some((environments, env_ref)) = uncommit {
        environments.remove(&env_ref);
    }
}

pub(super) fn cleanup_removed_entries_once(
    removed: &mut [(String, super::subagent_registry::SubagentEntry)],
) {
    for (_id, entry) in removed.iter_mut() {
        if let Some(env) = entry.cleanup_environment_id.take() {
            let argv = std::mem::take(&mut entry.cleanup_argv);
            run_cleanup_sync_once(Some(env), &argv);
        }
        uncommit_environment(take_environment_uncommit(entry));
    }
}

fn run_cleanup_sync_once(env_ref: Option<String>, argv: &[String]) {
    let Some(env_ref) = env_ref else {
        return;
    };
    if argv.is_empty() {
        return;
    }
    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", env_ref);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let _ = cmd.status();
}
