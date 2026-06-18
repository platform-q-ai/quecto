use super::*;
use quecto::domain::workflow::{WorkflowConfig, WorkflowEngine};
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, WorkflowSnapshot, new_registry,
};
use quecto::infrastructure::tools::workflow_tool::{broadcast_emitter, snapshot_to_event};
use quecto::interface::cli::protocol::build_subagent_info_list;
use std::path::PathBuf;

// ─── Stage B / R-B1: identity on workflow_state events ──────────────────────

#[given(expr = "a workflow agent {string} with no parent")]
fn given_workflow_agent_no_parent(world: &mut QuectoWorld, agent_id: String) {
    world.event_identity_agent_id = Some(agent_id);
    world.event_identity_parent_id = None;
}

#[when("the agent emits a workflow_state event")]
fn when_agent_emits_workflow_state(world: &mut QuectoWorld) {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let emitter = broadcast_emitter(
        tx,
        world.event_identity_agent_id.clone(),
        world.event_identity_parent_id.clone(),
    );
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    emitter(snapshot_to_event(&engine.snapshot(true)));
    let line = rx
        .try_recv()
        .expect("emitter should have sent one event line");
    world.event_identity_last = Some(serde_json::from_str(&line).expect("event is valid JSON"));
}

#[then(expr = "the workflow_state event should include agent_id {string}")]
fn then_event_includes_agent_id(world: &mut QuectoWorld, expected: String) {
    let ev = world
        .event_identity_last
        .as_ref()
        .expect("no event captured");
    assert_eq!(
        ev["agent_id"].as_str(),
        Some(expected.as_str()),
        "expected agent_id '{expected}', got: {ev}"
    );
}

#[then("the workflow_state event should include a parent_id field")]
fn then_event_includes_parent_id_field(world: &mut QuectoWorld) {
    let ev = world
        .event_identity_last
        .as_ref()
        .expect("no event captured");
    assert!(
        ev.as_object().is_some_and(|o| o.contains_key("parent_id")),
        "event should carry a parent_id field (null at the root): {ev}"
    );
}

// ─── Stage B / R-B3: SubagentInfo carries parent_id + workflow snapshot ─────

#[given(expr = "a parent agent {string}")]
fn given_parent_agent(world: &mut QuectoWorld, parent: String) {
    world.event_identity_agent_id = Some(parent);
    world.subagent_protocol_registry = Some(new_registry());
}

#[given(
    expr = "a subagent {string} spawned by {string} running a workflow at {int} of {int} steps"
)]
fn given_subagent_with_workflow(
    world: &mut QuectoWorld,
    child: String,
    parent: String,
    done: u32,
    total: u32,
) {
    let registry = world
        .subagent_protocol_registry
        .get_or_insert_with(new_registry);
    let mut entry = SubagentEntry::new(PathBuf::from(format!("/tmp/{child}.sock")), 0);
    entry.status = SubagentStatus::Running;
    entry.parent_id = Some(parent);
    entry.workflow = Some(WorkflowSnapshot {
        mode: "active".into(),
        steps_completed: done,
        steps_total: total,
    });
    registry.lock().unwrap().insert(child, entry);
}

#[when("the parent builds its subagent info list")]
fn when_build_subagent_info_list(world: &mut QuectoWorld) {
    world.subagent_infos = build_subagent_info_list(&world.subagent_protocol_registry);
}

#[then(expr = "the subagent entry for {string} should include parent_id {string}")]
fn then_subagent_parent_id(world: &mut QuectoWorld, child: String, expected: String) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == child)
        .expect("subagent entry for child");
    assert_eq!(
        info.parent_id.as_deref(),
        Some(expected.as_str()),
        "expected parent_id '{expected}'"
    );
}

#[then(
    expr = "the subagent entry for {string} should include a workflow snapshot of {int} of {int} steps"
)]
fn then_subagent_workflow_snapshot(world: &mut QuectoWorld, child: String, done: u32, total: u32) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == child)
        .expect("subagent entry for child");
    let wf = info.workflow.as_ref().expect("workflow snapshot present");
    assert_eq!(wf.steps_completed, done);
    assert_eq!(wf.steps_total, total);
}
