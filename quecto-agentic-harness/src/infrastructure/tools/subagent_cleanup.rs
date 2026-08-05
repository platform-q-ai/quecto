use super::subagent_registry::SubagentRegistry;

pub(super) async fn cleanup_registered_once(registry: &SubagentRegistry, agent_id: &str) {
    let cleanup_plan = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        entries.get_mut(agent_id).and_then(|entry| {
            entry
                .cleanup_environment_id
                .take()
                .map(|env| (env, std::mem::take(&mut entry.cleanup_argv)))
        })
    };
    if let Some((env, mut argv)) = cleanup_plan {
        super::spawn_container::run_cleanup_once(Some(env), &mut argv).await;
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
    }
}

pub(super) fn cleanup_all_before_clear_once(registry: &SubagentRegistry) -> usize {
    let mut removed: Vec<_> = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        entries.drain().collect()
    };
    let count = removed.len();
    cleanup_removed_entries_once(&mut removed);
    count
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
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_REF", env_ref);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let _ = cmd.status();
}
