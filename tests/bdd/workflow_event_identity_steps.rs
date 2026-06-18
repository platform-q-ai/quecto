use super::*;
use quecto::domain::workflow::{WorkflowConfig, WorkflowEngine};
use quecto::infrastructure::tools::workflow_tool::{broadcast_emitter, snapshot_to_event};

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
