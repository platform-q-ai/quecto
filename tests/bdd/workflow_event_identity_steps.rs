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

// ─── Stage B / R-B2: child workflow_state forwarded to the parent stream ────

#[given(expr = "a parent agent {string} with subagent {string}")]
fn given_parent_with_subagent(world: &mut QuectoWorld, parent: String, child: String) {
    world.event_identity_parent_id = Some(parent);
    world.event_identity_agent_id = Some(child);
}

#[when(expr = "{string} advances its workflow")]
fn when_child_advances_workflow(world: &mut QuectoWorld, child: String) {
    use quecto::infrastructure::tools::subagent_monitor::forward_child_workflow_event;
    // The child emits its own workflow_state line; the parent's monitor
    // re-stamps it with the child's identity before forwarding.
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    let child_line = serde_json::to_string(&snapshot_to_event(&engine.snapshot(true))).unwrap();
    let parent = world.event_identity_parent_id.clone();
    let forwarded = forward_child_workflow_event(&child_line, &child, parent.as_deref())
        .expect("a workflow_state line should be forwarded");
    world.event_identity_last = Some(serde_json::from_str(&forwarded).unwrap());
}

#[then(
    expr = "{string}'s event stream should receive a workflow_state event tagged agent_id {string} parent_id {string}"
)]
fn then_parent_stream_receives_tagged_event(
    world: &mut QuectoWorld,
    _parent_stream: String,
    expected_agent: String,
    expected_parent: String,
) {
    let ev = world
        .event_identity_last
        .as_ref()
        .expect("no forwarded event");
    assert_eq!(ev["type"].as_str(), Some("workflow_state"));
    assert_eq!(ev["agent_id"].as_str(), Some(expected_agent.as_str()));
    assert_eq!(ev["parent_id"].as_str(), Some(expected_parent.as_str()));
}

// ─── Stage B / R-B4: reconstruct the unit tree from the event stream ────────

fn tagged_event(agent: &str, parent: Option<&str>) -> serde_json::Value {
    serde_json::json!({ "type": "workflow_state", "agent_id": agent, "parent_id": parent })
}

#[given(
    expr = "an event stream with identity-tagged workflow_state events for {string}, {string} under {string}, and {string} under {string}"
)]
fn given_identity_tagged_stream(
    world: &mut QuectoWorld,
    root: String,
    child: String,
    child_parent: String,
    grandchild: String,
    grandchild_parent: String,
) {
    world.event_identity_stream = vec![
        tagged_event(&root, None),
        tagged_event(&child, Some(&child_parent)),
        tagged_event(&grandchild, Some(&grandchild_parent)),
    ];
}

#[when("a consumer reconstructs the unit tree from the stream")]
fn when_reconstruct_unit_tree(world: &mut QuectoWorld) {
    use quecto::interface::cli::protocol::UnitTree;
    let tree = UnitTree::from_events(&world.event_identity_stream);
    // Project the parentage into a JSON map so the assertion needn't import the type.
    let mut map = serde_json::Map::new();
    for ev in &world.event_identity_stream {
        if let Some(agent) = ev["agent_id"].as_str() {
            map.insert(agent.to_string(), serde_json::json!(tree.parent_of(agent)));
        }
    }
    world.event_identity_last = Some(serde_json::Value::Object(map));
}

#[then(expr = "the tree should place {string} under {string} under {string}")]
fn then_tree_places_nesting(
    world: &mut QuectoWorld,
    grandchild: String,
    child: String,
    root: String,
) {
    let map = world
        .event_identity_last
        .as_ref()
        .expect("no reconstructed tree");
    assert_eq!(
        map[&grandchild].as_str(),
        Some(child.as_str()),
        "grandchild parent"
    );
    assert_eq!(map[&child].as_str(), Some(root.as_str()), "child parent");
    assert!(map[&root].is_null(), "root should have no parent");
}
