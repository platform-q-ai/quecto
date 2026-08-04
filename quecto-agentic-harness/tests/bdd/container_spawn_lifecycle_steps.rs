use std::collections::BTreeMap;

use cucumber::{given, then, when};
use quecto::infrastructure::tools::container_registry::{
    ContainerEntry, ContainerStatus, list_containers, new_container_registry, register_container,
};
use serde_json::json;

use crate::QuectoWorld;

#[derive(Debug, Clone)]
struct SeamEvidence {
    local: serde_json::Value,
    container: serde_json::Value,
}

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

fn seam_evidence(world: &QuectoWorld) -> SeamEvidence {
    SeamEvidence {
        local: serde_json::from_str(&world.stdout).expect("local lifecycle seam evidence is JSON"),
        container: serde_json::from_str(&world.stderr)
            .expect("container lifecycle seam evidence is JSON"),
    }
}

fn assert_eq(world: &QuectoWorld, seam: &str, detail: &str) {
    let evidence = seam_evidence(world);
    assert_eq!(
        evidence.container[seam][detail], evidence.local[seam][detail],
        "container-backed supervision seam {seam}.{detail} must match local subagent behavior"
    );
}

#[given("a local subagent has completed a workflow run with transcript history")]
fn local_subagent_completed_workflow_run(world: &mut QuectoWorld) {
    world.stdout = json!({
        "messages_transcript": {
            "agent_cmd_messages": {"command": "get_messages", "messageRefs": ["m-user", "m-assistant"], "snapshot": false},
            "transcript_sync": {"command": "sync", "epoch": 7, "sinceRev": 0, "revisions": [1, 2]}
        },
        "workflow_status": {
            "workflow_state": {"event": "workflow_state", "mode": "active", "activeIssue": 1369},
            "status": {"event": "subagent_state_changed", "status": "idle", "agent_id": "local-ac7"}
        },
        "commands": {
            "kill": {"command": "kill", "success": true, "removed": true},
            "await": {"command": "await", "status": "completed", "consumedCompletion": true},
            "cleanup": {"command": "cleanup", "removed": true, "processReaped": true}
        }
    }).to_string();
}

#[given("a container-backed subagent has completed the same workflow run with transcript history")]
fn container_subagent_completed_workflow_run(world: &mut QuectoWorld) {
    world.stderr = json!({
        "messages_transcript": {
            "agent_cmd_messages": {"command": "get_messages", "messageRefs": [], "snapshot": true},
            "transcript_sync": {"command": "sync", "epoch": 0, "sinceRev": 0, "revisions": []}
        },
        "workflow_status": {
            "workflow_state": serde_json::Value::Null,
            "status": {"event": "subagent_state_changed", "status": "running", "agent_id": "container-ac7"}
        },
        "commands": {
            "kill": {"command": "kill", "success": false, "removed": false},
            "await": {"command": "await", "status": "timeout", "consumedCompletion": false},
            "cleanup": {"command": "cleanup", "removed": false, "processReaped": false}
        }
    }).to_string();
}

#[when("the parent compares lifecycle controls for both subagents")]
fn parent_compares_lifecycle_controls(_world: &mut QuectoWorld) {}

#[then("agent_cmd messages and transcript sync are equivalent")]
fn messages_and_transcript_are_equivalent(world: &mut QuectoWorld) {
    let evidence = seam_evidence(world);
    assert_eq!(
        evidence.container["messages_transcript"]["agent_cmd_messages"],
        evidence.local["messages_transcript"]["agent_cmd_messages"]
    );
    assert_eq(world, "messages_transcript", "agent_cmd_messages");
    assert_eq(world, "messages_transcript", "transcript_sync");
}

#[then("workflow state and status updates are equivalent")]
fn workflow_state_and_status_are_equivalent(world: &mut QuectoWorld) {
    let evidence = seam_evidence(world);
    assert_eq!(
        evidence.container["workflow_status"]["workflow_state"],
        evidence.local["workflow_status"]["workflow_state"]
    );
    assert_eq(world, "workflow_status", "workflow_state");
    assert_eq(world, "workflow_status", "status");
}

#[then("kill, await, and cleanup commands are equivalent")]
fn command_lifecycle_controls_are_equivalent(world: &mut QuectoWorld) {
    let evidence = seam_evidence(world);
    assert_eq!(
        evidence.container["commands"]["kill"],
        evidence.local["commands"]["kill"]
    );
    assert_eq(world, "commands", "kill");
    assert_eq(world, "commands", "await");
    assert_eq(world, "commands", "cleanup");
}

#[given("a container create script reports environment metadata")]
fn container_create_script_reports_metadata(world: &mut QuectoWorld) {
    world.stdout = json!({
        "runtime": "opaque-script-runtime",
        "labels": {"team": "agent-platform", "cost-center": "r-and-d"},
        "nested": {"preserve": ["all", "script", "fields"], "number": 42, "flag": true},
        "arbitrary": [{"k": "v"}, null, 3]
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
    world.stderr = json!({
        "expected": script_metadata,
        "recorded": recorded.metadata,
        "all": list_containers(&registry).into_iter().map(|entry| entry.metadata).collect::<Vec<_>>()
    }).to_string();
}

#[then("the recorded container metadata exactly matches the script output")]
fn recorded_metadata_matches_script_output(world: &mut QuectoWorld) {
    let comparison: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(
        comparison["recorded"], comparison["expected"],
        "container registry must retain arbitrary script-reported metadata verbatim"
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
    world.stdout = json!({
        "pushed_liveness_events": [],
        "status": "running",
        "pending_await": true,
        "inspect_invocations": [],
        "poll_attempts": 0
    })
    .to_string();
}

#[when("the liveness connection receives EOF")]
fn liveness_connection_receives_eof(world: &mut QuectoWorld) {
    let before: serde_json::Value = serde_json::from_str(&world.stdout).unwrap();
    let mut inspect_invocation = BTreeMap::new();
    inspect_invocation.insert("source", "poll-loop");
    world.stderr = json!({
        "pushed_liveness_events": before["pushed_liveness_events"],
        "status": before["status"],
        "pending_await": before["pending_await"],
        "inspect_invocations": [inspect_invocation],
        "poll_attempts": 1
    })
    .to_string();
}

#[then("EOF is treated as a pushed liveness signal")]
fn eof_is_treated_as_pushed_liveness_signal(world: &mut QuectoWorld) {
    let after: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(after["pushed_liveness_events"], json!(["eof"]));
}

#[then("the subagent is marked exited from the pushed liveness signal")]
fn subagent_marked_exited_from_pushed_signal(world: &mut QuectoWorld) {
    let after: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(after["status"], "exited");
}

#[then("exactly one post-mortem inspect is requested")]
fn exactly_one_post_mortem_inspect(world: &mut QuectoWorld) {
    let after: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(after["inspect_invocations"].as_array().unwrap().len(), 1);
    assert_eq!(after["inspect_invocations"][0]["source"], "eof-post-mortem");
}

#[then("the pending await completes without polling")]
fn pending_await_completes_without_polling(world: &mut QuectoWorld) {
    let after: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(after["pending_await"], false);
    assert_eq!(after["poll_attempts"], 0);
}
