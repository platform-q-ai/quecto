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

/// #2471: a child gets a launcher board only when it is a swarm worker (it
/// holds a board member) and its launcher created its run.
#[test]
fn only_a_run_creator_s_swarm_workers_settle_their_turn_ends_by_its_board() {
    let context = || {
        Some(crate::infrastructure::tools::swarm_bridge::SwarmContext {
            board: crate::composition::swarm::swarm_board(),
            checkout: std::env::temp_dir(),
            member: "member-1".into(),
            lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        })
    };
    use crate::infrastructure::tools::swarm_bridge::Participation;
    let launcher = |participation| {
        let (tool, _socket_dir) = tool();
        tool.with_swarm_context(context())
            .with_swarm_participation(participation)
    };
    let created = Participation::shared();
    created.record_creator(true);
    created.set(true);
    let creator = launcher(created);
    let board = super::worker_launcher_board(&creator, Some("m-1".into()));
    assert_eq!(board.map(|board| board.member), Some("m-1".to_owned()));
    assert!(super::worker_launcher_board(&creator, None).is_none());
    let worker = launcher(Participation::Fixed(true));
    assert!(super::worker_launcher_board(&worker, Some("m-1".into())).is_none());
}

/// #2471: registering a swarm worker launched by its run's creator gives
/// its entry the launcher's board under the worker's member id; any other
/// launcher's worker keeps the ordinary notes.
#[tokio::test]
async fn register_gives_a_run_creator_s_worker_the_launcher_board() {
    for (created_run, expected, task) in [
        (true, Some("m-1"), None),
        (true, Some("m-1"), Some("claim a task")),
        (false, None, None),
    ] {
        let mut launch = config();
        launch.task = task.map(str::to_owned);
        let context = crate::infrastructure::tools::swarm_bridge::SwarmContext {
            board: crate::composition::swarm::swarm_board(),
            checkout: std::env::temp_dir(),
            member: "coordinator".into(),
            lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        };
        let participation = crate::infrastructure::tools::swarm_bridge::Participation::shared();
        participation.record_creator(created_run);
        participation.set(true);
        let (tool, _socket_dir) = tool();
        let tool = tool
            .with_swarm_context(Some(context.clone()))
            .with_swarm_participation(participation);
        let mut ports = SpawnLaunchPorts::new(&tool);
        let identity = ports.allocate_identity(&config()).unwrap();
        let mut prepared =
            crate::infrastructure::tools::spawn_container::PreparedChild::new_for_test(
                None, None, None,
            )
            .await;
        prepared.swarm_reservation = Some(
            crate::infrastructure::tools::swarm_admission::LaunchReservation::held_for_test(
                context, "m-1",
            ),
        );
        let runtime = PreparedRuntime {
            socket_path: std::path::PathBuf::from("/tmp/worker-2471.sock"),
            pid: 0,
            environment_ref: None,
        };
        ports
            .register_and_monitor(&identity, runtime, &mut prepared, &launch)
            .await
            .unwrap();
        let member = tool
            .registry
            .lock()
            .unwrap()
            .get(&identity.registry_key)
            .and_then(|entry| entry.coordinator_wake.worker.launcher_board.clone())
            .map(|board| board.member);
        assert_eq!(member.as_deref(), expected, "created_run {created_run}");
        let task_pending = tool
            .registry
            .lock()
            .unwrap()
            .get(&identity.registry_key)
            .is_some_and(|entry| entry.coordinator_wake.worker.task_pending);
        assert_eq!(
            task_pending,
            created_run && task.is_some(),
            "a run creator's worker launched with a task"
        );
    }
}
