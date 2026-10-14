//! S4c: the launcher reads spawn's running line back as it spawn writes
//! it, and asks spawn for a coordinator in a new container.
use super::super::spawn_launch_ports::running_line;
use super::*;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::Effort;

#[test]
fn spawns_running_line_reads_back_as_the_coordinator_it_names() {
    let details = " environment_ref=C12 container_config=standard workspace=/w";
    let line = running_line("board-a-run-1", "7f3a-uuid", details);
    assert_eq!(
        launched(&format!("{line}\nConfiguration diagnostics:\nan overlay")),
        Some(LaunchedCoordinator {
            agent: "7f3a-uuid".into(),
            environment_ref: "C12".into(),
        })
    );
    let bare = running_line("board-a-run-1", "7f3a-uuid", " environment_ref=C3");
    assert_eq!(
        launched(&bare).map(|it| it.environment_ref),
        Some("C3".into())
    );
    // A local child names no container: no coordinator runs.
    assert_eq!(launched(&running_line("x", "7f3a-uuid", "")), None);
}

#[test]
fn a_coordinator_is_spawned_in_a_new_container_with_its_role() {
    let request = CoordinatorRequest {
        task: Slug::parse("board-store").unwrap(),
        run: 2,
        plan_version: 3,
        brief: "the brief".into(),
        model: Some("anthropic/claude-opus".into()),
        effort: Some(Effort::High),
    };
    assert_eq!(
        spawn_arguments(&request),
        json!({"agent_id": "board-board-store-run-2", "task": "the brief", "container": true,
               "coordinator": true, "model": "anthropic/claude-opus", "effort": "high"})
    );
    let unnamed = CoordinatorRequest {
        model: None,
        effort: None,
        ..request
    };
    let arguments = spawn_arguments(&unnamed);
    assert!(arguments.get("model").is_none() && arguments.get("effort").is_none());
}
