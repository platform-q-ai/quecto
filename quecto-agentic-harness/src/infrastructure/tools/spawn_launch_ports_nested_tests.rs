use super::*;
use std::sync::Arc;

/// The tool and the socket directory it launches into: the caller holds
/// the directory, so it is removed when the test ends.
fn tool() -> (SpawnTool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let tool = crate::composition::subagent_lifecycle::compose_launcher(
        SpawnTool::new(vec![]).with_socket_dir(dir.path().to_path_buf()),
    );
    (tool, dir)
}

fn config() -> SubagentConfig {
    SubagentConfig {
        agent_id: Some("worker".into()),
        system: None,
        task: None,
        model: None,
        effort: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        disable_tools: vec![],
        read_only: false,
        container: crate::domain::agents::services::subagent::ContainerSelection::Local,
        backend: Default::default(),
        coordinator: false,
    }
}

/// #2204: the launcher hands the child command its swarm participation, so
/// a nested container is refused in the swarm wording only for a process
/// that takes part in a created run.
#[tokio::test]
async fn a_nested_container_refusal_follows_the_launchers_swarm_participation() {
    for (participating, expected) in [
        (
            false,
            crate::infrastructure::tools::spawn_container::NESTED_CONTAINER_INSIDE_CONTAINER,
        ),
        (
            true,
            crate::infrastructure::tools::spawn_container::NESTED_CONTAINER_SWARM_MEMBER,
        ),
    ] {
        let (tool, _socket_dir) = tool();
        let tool = tool
            .with_swarm_context(Some(
                crate::infrastructure::tools::swarm_bridge::SwarmContext {
                    board: crate::composition::swarm::swarm_board(),
                    checkout: std::env::temp_dir(),
                    member: "member-1".into(),
                    lifecycle: Arc::new(crate::application::swarm::LifecycleService),
                },
            ))
            .with_swarm_participation(
                crate::infrastructure::tools::swarm_bridge::Participation::Fixed(participating),
            );
        let mut container = config();
        container.container = crate::domain::agents::services::subagent::ContainerSelection::New {
            container_config: None,
            name: None,
        };
        let mut ports = SpawnLaunchPorts::new(&tool);
        ports.allocate_identity(&container).unwrap();
        let error = ports
            .prepare_child(&container, std::path::Path::new("true"), &[])
            .await
            .map(|_| ())
            .unwrap_err();
        assert_eq!(error.to_string(), format!("tool error: {expected}"));
    }
}
