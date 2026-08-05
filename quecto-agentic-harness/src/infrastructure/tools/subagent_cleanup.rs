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
