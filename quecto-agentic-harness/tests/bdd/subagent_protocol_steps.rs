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
fn given_registry_with_subagent(
    world: &mut QuectoWorld,
    agent_id: String,
    status: String,
    last_tool: String,
    pid: i32,
) {
    insert_registry_subagent(world, agent_id, status, last_tool, pid, false);
}

#[given(
    expr = "a registry with read-only subagent {string} status {string} last_tool {string} pid {int}"
)]
fn given_registry_with_readonly_subagent(
    world: &mut QuectoWorld,
    agent_id: String,
    status: String,
    last_tool: String,
    pid: i32,
) {
    insert_registry_subagent(world, agent_id, status, last_tool, pid, true);
}

#[given(expr = "a read-only observer sub-agent {string} is registered")]
fn given_registered_observer(world: &mut QuectoWorld, agent_id: String) {
    insert_registry_subagent(world, agent_id, "running".into(), "bash".into(), 1234, true);
}

#[given(expr = "a read-write sub-agent {string} is registered")]
fn given_registered_readwrite(world: &mut QuectoWorld, agent_id: String) {
    insert_registry_subagent(world, agent_id, "idle".into(), "".into(), 5678, false);
}

#[given(expr = "a registry subagent with display name {string} and hidden identity {string}")]
fn given_registry_subagent_with_dual_identity(
    world: &mut QuectoWorld,
    display_name: String,
    hidden_identity: String,
) {
    insert_registry_subagent_with_key(
        world,
        hidden_identity,
        display_name,
        "idle".into(),
        "".into(),
        1234,
        false,
    );
}

fn insert_registry_subagent(
    world: &mut QuectoWorld,
    agent_id: String,
    status: String,
    last_tool: String,
    pid: i32,
    read_only: bool,
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
    entry.read_only = read_only;
    if !last_tool.is_empty() {
        entry.last_tool = Some(last_tool);
    }
    let display_name = agent_id.clone();
    insert_registry_subagent_with_key_parts(&mut guard, agent_id, display_name, entry);
}

#[expect(
    clippy::too_many_arguments,
    reason = "BDD registry helper mirrors scenario data"
)]
fn insert_registry_subagent_with_key(
    world: &mut QuectoWorld,
    key: String,
    display_name: String,
    status: String,
    last_tool: String,
    pid: i32,
    read_only: bool,
) {
    if world.subagent_protocol_registry.is_none() {
        world.subagent_protocol_registry = Some(new_registry());
    }
    let reg = world.subagent_protocol_registry.as_ref().unwrap();
    let mut guard = reg.lock().unwrap();
    let mut entry = SubagentEntry::with_identity(
        quecto::domain::ids::AgentUuid::new(key.clone()),
        display_name.clone(),
        "/tmp/test.sock".into(),
        pid as u32,
    );
    entry.status = match status.as_str() {
        "starting" => SubagentStatus::Starting,
        "idle" => SubagentStatus::Idle,
        "running" => SubagentStatus::Running,
        "error" => SubagentStatus::Error,
        "exited" => SubagentStatus::Exited,
        _ => panic!("unknown status: {status}"),
    };
    entry.read_only = read_only;
    if !last_tool.is_empty() {
        entry.last_tool = Some(last_tool);
    }
    insert_registry_subagent_with_key_parts(&mut guard, key, display_name, entry);
}

fn insert_registry_subagent_with_key_parts(
    guard: &mut std::collections::HashMap<String, SubagentEntry>,
    key: String,
    display_name: String,
    mut entry: SubagentEntry,
) {
    entry.display_name = display_name;
    guard.insert(key, entry);
}

#[given(expr = "subagent {string} has last_error {string}")]
fn given_subagent_has_error(world: &mut QuectoWorld, agent_id: String, error: String) {
    let reg = world.subagent_protocol_registry.as_ref().unwrap();
    let mut guard = reg.lock().unwrap();
    let entry = guard.get_mut(&agent_id).expect("subagent not found");
    entry.last_error = Some(error);
}

#[when("I build a SubagentInfo list from the registry")]
#[when("a client requests sub-agent state")]
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

#[then(expr = "subagent info {string} should not have socketPath")]
fn then_subagent_socket_path_omitted(world: &mut QuectoWorld, agent_id: String) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == agent_id)
        .unwrap_or_else(|| panic!("subagent '{}' not found in list", agent_id));
    assert_eq!(
        info.socket_path.as_deref(),
        None,
        "socket_path should be omitted for {}",
        agent_id
    );
}

#[when("I serialize the first subagent info")]
fn when_round_trip_first_info(world: &mut QuectoWorld) {
    let first = world
        .subagent_infos
        .first()
        .expect("no subagent info to round-trip");
    world.subagent_info_json = serde_json::to_value(first).unwrap();
}

#[then(expr = "the round-tripped subagent info should not have socketPath")]
fn then_round_tripped_socket_path_omitted(world: &mut QuectoWorld) {
    let info: SubagentInfo =
        serde_json::from_value(world.subagent_info_json.clone()).expect("info should deserialize");
    assert_eq!(
        info.socket_path.as_deref(),
        None,
        "round-tripped socket_path should be omitted (json: {})",
        world.subagent_info_json
    );
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

#[then(expr = "subagent info {string} should be read-only")]
fn then_subagent_readonly(world: &mut QuectoWorld, agent_id: String) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == agent_id)
        .unwrap_or_else(|| panic!("subagent '{}' not found in list", agent_id));
    assert!(
        info.read_only,
        "subagent info for {agent_id} must identify a read-only observer"
    );
}

#[then(expr = "subagent info {string} should be read-write")]
fn then_subagent_readwrite(world: &mut QuectoWorld, agent_id: String) {
    let info = world
        .subagent_infos
        .iter()
        .find(|i| i.agent_id == agent_id)
        .unwrap_or_else(|| panic!("subagent '{}' not found in list", agent_id));
    assert!(
        !info.read_only,
        "subagent info for {agent_id} must not identify a read-write sub-agent as an observer"
    );
}

#[then("the subagent info list should contain both observer and read-write states")]
fn then_subagent_info_list_has_mixed_observer_states(world: &mut QuectoWorld) {
    assert!(
        world.subagent_infos.iter().any(|info| info.read_only),
        "expected at least one observer entry in subagent info list"
    );
    assert!(
        world.subagent_infos.iter().any(|info| !info.read_only),
        "expected at least one read-write entry in subagent info list"
    );
}

// ─── SubagentInfo serialization steps ─────────────────────────────────────────

#[given(expr = "a SubagentInfo with agentId {string} status {string} lastTool {string} pid {int}")]
fn given_subagent_info(
    world: &mut QuectoWorld,
    agent_id: String,
    status: String,
    last_tool: String,
    pid: i32,
) {
    world.subagent_info_single = Some(SubagentInfo {
        agent_uuid: None,
        display_name: None,
        agent_id,
        status,
        liveness: None,
        last_tool: if last_tool.is_empty() {
            None
        } else {
            Some(last_tool)
        },
        last_error: None,
        pid: pid as u32,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: "local".to_string(),
        environment: None,
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
            agent_uuid: None,
            display_name: None,
            agent_id: format!("agent-{i}"),
            status: "idle".to_string(),
            liveness: None,
            last_tool: None,
            last_error: None,
            pid: i as u32,
            socket_path: None,
            parent_id: None,
            workflow: None,
            read_only: false,
            execution_backend: "local".to_string(),
            environment: None,
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
            agent_uuid: None,
            display_name: None,
            agent_id,
            status,
            liveness: None,
            last_tool: None,
            last_error: None,
            pid: 1,
            socket_path: None,
            parent_id: None,
            workflow: None,
            read_only: false,
            execution_backend: "local".to_string(),
            environment: None,
        }],
    });
}

#[given(
    expr = "a SubagentStateChanged event for read-only sub-agent {string} and read-write sub-agent {string}"
)]
fn given_state_changed_observer_and_readwrite(
    world: &mut QuectoWorld,
    observer: String,
    readwrite: String,
) {
    world.protocol_event = Some(AgentEvent::SubagentStateChanged {
        subagents: vec![
            SubagentInfo {
                agent_uuid: None,
                display_name: None,
                agent_id: observer,
                status: "running".to_string(),
                liveness: None,
                last_tool: None,
                last_error: None,
                pid: 1,
                socket_path: None,
                parent_id: None,
                workflow: None,
                read_only: true,
                execution_backend: "local".to_string(),
                environment: None,
            },
            SubagentInfo {
                agent_uuid: None,
                display_name: None,
                agent_id: readwrite,
                status: "idle".to_string(),
                liveness: None,
                last_tool: None,
                last_error: None,
                pid: 2,
                socket_path: None,
                parent_id: None,
                workflow: None,
                read_only: false,
                execution_backend: "local".to_string(),
                environment: None,
            },
        ],
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
    let found = deserialized_subagent(world, &agent_id);
    assert_eq!(found.status, status);
}

#[then(expr = "the deserialized subagents should contain {string} as read-only")]
fn then_deserialized_readonly(world: &mut QuectoWorld, agent_id: String) {
    let found = deserialized_subagent(world, &agent_id);
    assert!(
        found.read_only,
        "deserialized sub-agent {agent_id} should preserve read-only observer status"
    );
}

#[then(expr = "the deserialized subagents should contain {string} as read-write")]
fn then_deserialized_readwrite(world: &mut QuectoWorld, agent_id: String) {
    let found = deserialized_subagent(world, &agent_id);
    assert!(
        !found.read_only,
        "deserialized sub-agent {agent_id} should preserve read-write status"
    );
}

fn deserialized_subagent<'a>(world: &'a QuectoWorld, agent_id: &str) -> &'a SubagentInfo {
    let ev = world.deserialized_event.as_ref().unwrap();
    match ev {
        AgentEvent::SubagentStateChanged { subagents } => subagents
            .iter()
            .find(|s| s.agent_id == agent_id)
            .unwrap_or_else(|| panic!("agent '{agent_id}' not found")),
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
