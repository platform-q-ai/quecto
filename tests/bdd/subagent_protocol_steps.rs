use super::*;
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};
use quecto::interface::cli::protocol::{
    AgentCommand, AgentEvent, SubagentInfo, build_subagent_info_list,
};

// ─── World fields ─────────────────────────────────────────────────────────────
// Uses: world.subagent_infos, world.subagent_info_json, world.parsed_command,
//       world.serialized_event_json, world.deserialized_event
// These are declared in main.rs.

// ─── Registry-based steps ─────────────────────────────────────────────────────

#[given("a SubagentInfo list from an empty registry")]
fn given_empty_registry(world: &mut QuectoWorld) {
    world.subagent_infos = build_subagent_info_list(&None);
}

#[given(expr = "a registry with subagent {string} status {string} last_tool {string} pid {int}")]
#[allow(clippy::too_many_arguments)]
fn given_registry_with_subagent(
    world: &mut QuectoWorld,
    agent_id: String,
    status: String,
    last_tool: String,
    pid: i32,
) {
    if world.subagent_protocol_registry.is_none() {
        world.subagent_protocol_registry = Some(new_registry());
    }
    let reg = world.subagent_protocol_registry.as_ref().unwrap();
    let mut guard = reg.lock().unwrap();
    let mut entry = SubagentEntry::new("/tmp/test.sock".into(), pid as u32);
    entry.status = match status.as_str() {
        "starting" => SubagentStatus::Starting,
        "idle" => SubagentStatus::Idle,
        "running" => SubagentStatus::Running,
        "error" => SubagentStatus::Error,
        "exited" => SubagentStatus::Exited,
        _ => panic!("unknown status: {status}"),
    };
    if !last_tool.is_empty() {
        entry.last_tool = Some(last_tool);
    }
    guard.insert(agent_id, entry);
}

#[given(expr = "subagent {string} has last_error {string}")]
fn given_subagent_has_error(world: &mut QuectoWorld, agent_id: String, error: String) {
    let reg = world.subagent_protocol_registry.as_ref().unwrap();
    let mut guard = reg.lock().unwrap();
    let entry = guard.get_mut(&agent_id).expect("subagent not found");
    entry.last_error = Some(error);
}

#[when("I build a SubagentInfo list from the registry")]
fn when_build_info_list(world: &mut QuectoWorld) {
    world.subagent_infos = build_subagent_info_list(&world.subagent_protocol_registry);
}

#[then("the subagent info list should be empty")]
fn then_list_empty(world: &mut QuectoWorld) {
    assert!(world.subagent_infos.is_empty(), "expected empty list");
}

#[then(expr = "the subagent info list should have {int} entries")]
fn then_list_count(world: &mut QuectoWorld, count: usize) {
    assert_eq!(
        world.subagent_infos.len(),
        count,
        "expected {} entries, got {}",
        count,
        world.subagent_infos.len()
    );
}

#[then(expr = "subagent info {string} should have status {string}")]
fn then_subagent_status(world: &mut QuectoWorld, agent_id: String, status: String) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == agent_id)
        .unwrap_or_else(|| panic!("subagent '{}' not found in list", agent_id));
    assert_eq!(info.status, status, "status mismatch for {}", agent_id);
}

#[then(expr = "subagent info {string} should have last_tool {string}")]
fn then_subagent_last_tool(world: &mut QuectoWorld, agent_id: String, tool: String) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == agent_id)
        .unwrap_or_else(|| panic!("subagent '{}' not found in list", agent_id));
    assert_eq!(
        info.last_tool.as_deref(),
        Some(tool.as_str()),
        "last_tool mismatch for {}",
        agent_id
    );
}

#[then(expr = "subagent info {string} should have pid {int}")]
fn then_subagent_pid(world: &mut QuectoWorld, agent_id: String, pid: i32) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == agent_id)
        .unwrap_or_else(|| panic!("subagent '{}' not found in list", agent_id));
    assert_eq!(info.pid, pid as u32, "pid mismatch for {}", agent_id);
}

#[then(expr = "subagent info {string} should have last_error {string}")]
fn then_subagent_last_error(world: &mut QuectoWorld, agent_id: String, error: String) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == agent_id)
        .unwrap_or_else(|| panic!("subagent '{}' not found in list", agent_id));
    assert_eq!(
        info.last_error.as_deref(),
        Some(error.as_str()),
        "last_error mismatch for {}",
        agent_id
    );
}

// ─── SubagentInfo serialization steps ─────────────────────────────────────────

#[given(expr = "a SubagentInfo with agentId {string} status {string} lastTool {string} pid {int}")]
#[allow(clippy::too_many_arguments)]
fn given_subagent_info(
    world: &mut QuectoWorld,
    agent_id: String,
    status: String,
    last_tool: String,
    pid: i32,
) {
    world.subagent_info_single = Some(SubagentInfo {
        agent_id,
        status,
        last_tool: if last_tool.is_empty() {
            None
        } else {
            Some(last_tool)
        },
        last_error: None,
        pid: pid as u32,
    });
}

#[given(expr = "the SubagentInfo has lastError {string}")]
fn given_subagent_info_has_error(world: &mut QuectoWorld, error: String) {
    world.subagent_info_single.as_mut().unwrap().last_error = Some(error);
}

#[when("I serialize the SubagentInfo to JSON")]
fn when_serialize_info(world: &mut QuectoWorld) {
    let info = world.subagent_info_single.as_ref().unwrap();
    world.subagent_info_json = serde_json::to_value(info).unwrap();
}

#[then(expr = "the JSON should contain key {string} with value {string}")]
fn then_json_key_string(world: &mut QuectoWorld, key: String, value: String) {
    let json = &world.subagent_info_json;
    assert_eq!(
        json[&key].as_str().unwrap(),
        value,
        "key '{}' mismatch",
        key
    );
}

#[then(expr = "the JSON should contain key {string} with value {int}")]
fn then_json_key_int(world: &mut QuectoWorld, key: String, value: i64) {
    let json = &world.subagent_info_json;
    assert_eq!(
        json[&key].as_i64().unwrap(),
        value,
        "key '{}' mismatch",
        key
    );
}

#[then(expr = "the JSON should contain key {string} with null value")]
fn then_json_key_null(world: &mut QuectoWorld, key: String) {
    let json = &world.subagent_info_json;
    assert!(
        json.get(&key).is_none() || json[&key].is_null(),
        "expected key '{}' to be absent or null, got {:?}",
        key,
        json.get(&key)
    );
}

// ─── Command parsing steps ──────────────────────────────────────────────────

#[given(expr = "the JSON command {string}")]
fn given_json_command(world: &mut QuectoWorld, json: String) {
    world.protocol_command_json = json;
}

#[when("I parse the command")]
fn when_parse_command(world: &mut QuectoWorld) {
    let cmd: AgentCommand =
        serde_json::from_str(&world.protocol_command_json).expect("failed to parse command");
    world.parsed_command = Some(cmd);
}

#[then(expr = "the command type should be {string}")]
fn then_command_type(world: &mut QuectoWorld, expected: String) {
    let cmd = world.parsed_command.as_ref().unwrap();
    assert_eq!(cmd.type_name(), expected);
}

#[then(expr = "the command id should be {string}")]
fn then_command_id(world: &mut QuectoWorld, expected: String) {
    let cmd = world.parsed_command.as_ref().unwrap();
    assert_eq!(cmd.id(), Some(expected.as_str()));
}

#[then("the command id should be absent")]
fn then_command_id_absent(world: &mut QuectoWorld) {
    let cmd = world.parsed_command.as_ref().unwrap();
    assert!(cmd.id().is_none());
}

// ─── subagent_state_changed event steps ─────────────────────────────────────

#[given(expr = "a SubagentStateChanged event with {int} subagents")]
fn given_state_changed_event(world: &mut QuectoWorld, count: usize) {
    let subagents: Vec<SubagentInfo> = (0..count)
        .map(|i| SubagentInfo {
            agent_id: format!("agent-{i}"),
            status: "idle".to_string(),
            last_tool: None,
            last_error: None,
            pid: i as u32,
        })
        .collect();
    world.protocol_event = Some(AgentEvent::SubagentStateChanged { subagents });
}

#[given(expr = "a SubagentStateChanged event with {int} subagent {string} status {string}")]
fn given_state_changed_one(
    world: &mut QuectoWorld,
    _count: usize,
    agent_id: String,
    status: String,
) {
    world.protocol_event = Some(AgentEvent::SubagentStateChanged {
        subagents: vec![SubagentInfo {
            agent_id,
            status,
            last_tool: None,
            last_error: None,
            pid: 1,
        }],
    });
}

#[when("I serialize the event to JSON")]
fn when_serialize_event(world: &mut QuectoWorld) {
    let ev = world.protocol_event.as_ref().unwrap();
    world.subagent_info_json = serde_json::from_str(&ev.to_json_line()).unwrap();
}

#[then(expr = "the JSON should contain {string} with value {string}")]
fn then_json_contains_string(world: &mut QuectoWorld, key: String, value: String) {
    let json = &world.subagent_info_json;
    assert_eq!(json[&key].as_str().unwrap(), value);
}

#[then(expr = "the JSON should contain a {string} array with {int} entries")]
fn then_json_array_count(world: &mut QuectoWorld, key: String, count: usize) {
    let json = &world.subagent_info_json;
    let arr = json[&key].as_array().unwrap();
    assert_eq!(arr.len(), count);
}

#[when("I serialize and deserialize the event")]
fn when_roundtrip_event(world: &mut QuectoWorld) {
    let ev = world.protocol_event.as_ref().unwrap();
    let json = ev.to_json_line();
    world.deserialized_event = Some(serde_json::from_str(&json).unwrap());
}

#[then("the deserialized event should be SubagentStateChanged")]
fn then_event_is_state_changed(world: &mut QuectoWorld) {
    let ev = world.deserialized_event.as_ref().unwrap();
    assert!(
        matches!(ev, AgentEvent::SubagentStateChanged { .. }),
        "expected SubagentStateChanged"
    );
}

#[then(expr = "the deserialized subagents should contain {string} with status {string}")]
fn then_deserialized_contains(world: &mut QuectoWorld, agent_id: String, status: String) {
    let ev = world.deserialized_event.as_ref().unwrap();
    match ev {
        AgentEvent::SubagentStateChanged { subagents } => {
            let found = subagents
                .iter()
                .find(|s| s.agent_id == agent_id)
                .unwrap_or_else(|| panic!("agent '{}' not found", agent_id));
            assert_eq!(found.status, status);
        }
        _ => panic!("expected SubagentStateChanged"),
    }
}

// ─── Sorting steps ──────────────────────────────────────────────────────────

#[then(expr = "the first subagent info should have agentId {string}")]
fn then_first_agent_id(world: &mut QuectoWorld, expected: String) {
    assert_eq!(world.subagent_infos[0].agent_id, expected);
}

#[then(expr = "the second subagent info should have agentId {string}")]
fn then_second_agent_id(world: &mut QuectoWorld, expected: String) {
    assert_eq!(world.subagent_infos[1].agent_id, expected);
}
