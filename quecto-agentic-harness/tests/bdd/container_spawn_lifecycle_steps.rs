use cucumber::{given, then, when};
use quecto::infrastructure::tools::container_registry::{
    ContainerEntry, ContainerStatus, list_containers, new_container_registry, register_container,
};

use crate::QuectoWorld;

const PARITY_CONTROLS: &[&str] = &[
    "agent_cmd messages",
    "workflow state",
    "status",
    "transcript",
    "kill",
    "await",
    "cleanup",
];

fn lifecycle_entry(metadata: serde_json::Value, status: ContainerStatus) -> ContainerEntry {
    ContainerEntry {
        container_uuid: "env-ac8".into(),
        container_ref: String::new(),
        container_name: Some("script-name".into()),
        environment_id: "script-environment".into(),
        repo_url: Some("https://example.invalid/repo.git".into()),
        workspace_path: "/workspace/script".into(),
        status,
        agents: vec![],
        metadata,
    }
}

#[given("a local subagent has completed a workflow run with transcript history")]
fn local_subagent_completed_workflow_run(world: &mut QuectoWorld) {
    world.stdout = PARITY_CONTROLS.join("|");
}

#[given("a container-backed subagent has completed the same workflow run with transcript history")]
fn container_subagent_completed_workflow_run(world: &mut QuectoWorld) {
    // RED AC7: there is not yet a container-backed lifecycle protocol surface proving
    // these controls are backed by the same parent APIs as local subagents.
    world.stderr = "status|transcript".into();
}

#[when("the parent compares lifecycle controls for both subagents")]
fn parent_compares_lifecycle_controls(_world: &mut QuectoWorld) {}

#[then(
    "agent_cmd messages, workflow state, status, transcript, kill, await, and cleanup are equivalent"
)]
fn lifecycle_controls_are_equivalent(world: &mut QuectoWorld) {
    let local: Vec<_> = world.stdout.split('|').collect();
    let container: Vec<_> = world.stderr.split('|').collect();
    assert_eq!(
        container, local,
        "container-backed spawn must expose every local lifecycle control through protocol parity"
    );
}

#[given("a container create script reports environment metadata")]
fn container_create_script_reports_metadata(world: &mut QuectoWorld) {
    world.stdout = serde_json::json!({
        "runtime": "opaque-script-runtime",
        "labels": {"team": "agent-platform"},
        "nested": {"preserve": ["all", "script", "fields"]}
    })
    .to_string();
}

#[when("the container entry is recorded")]
fn container_entry_is_recorded(world: &mut QuectoWorld) {
    let script_metadata: serde_json::Value = serde_json::from_str(&world.stdout).unwrap();
    let registry = new_container_registry();
    let recorded = register_container(
        &registry,
        lifecycle_entry(serde_json::json!({}), ContainerStatus::Running),
    );
    world.stderr = serde_json::json!({
        "expected": script_metadata,
        "recorded": recorded.metadata,
        "all": list_containers(&registry).into_iter().map(|entry| entry.metadata).collect::<Vec<_>>()
    })
    .to_string();
}

#[then("the recorded container metadata exactly matches the script output")]
fn recorded_metadata_matches_script_output(world: &mut QuectoWorld) {
    let comparison: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(
        comparison["recorded"], comparison["expected"],
        "container registry must retain the script-reported metadata verbatim"
    );
}

#[then("no Docker or runtime-specific fields are inferred by Quecto core")]
fn no_runtime_specific_fields_are_inferred(world: &mut QuectoWorld) {
    let comparison: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    let recorded = comparison["recorded"].as_object().unwrap();
    for forbidden in ["docker", "image", "container_id", "runtime_inferred"] {
        assert!(
            !recorded.contains_key(forbidden),
            "Quecto core must not infer runtime-specific metadata field {forbidden}"
        );
    }
}

#[given("a container-backed subagent has a liveness connection and a pending await")]
fn container_subagent_has_liveness_and_pending_await(world: &mut QuectoWorld) {
    world.stdout = serde_json::json!({
        "status": "running",
        "pending_await": true,
        "inspect_count": 0,
        "completed_by_polling": false
    })
    .to_string();
}

#[when("the liveness connection receives EOF")]
fn liveness_connection_receives_eof(world: &mut QuectoWorld) {
    let before: serde_json::Value = serde_json::from_str(&world.stdout).unwrap();
    // RED AC9: socket EOF is not yet wired to status, one-shot inspect, or await completion.
    world.stderr = serde_json::json!({
        "status": before["status"],
        "pending_await": before["pending_await"],
        "inspect_count": before["inspect_count"],
        "completed_by_polling": true
    })
    .to_string();
}

#[then("the subagent is marked exited from the pushed liveness signal")]
fn subagent_marked_exited_from_pushed_signal(world: &mut QuectoWorld) {
    let after: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(after["status"], "exited");
}

#[then("exactly one post-mortem inspect is requested")]
fn exactly_one_post_mortem_inspect(world: &mut QuectoWorld) {
    let after: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(after["inspect_count"], 1);
}

#[then("the pending await completes without polling")]
fn pending_await_completes_without_polling(world: &mut QuectoWorld) {
    let after: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(after["pending_await"], false);
    assert_eq!(after["completed_by_polling"], false);
}
