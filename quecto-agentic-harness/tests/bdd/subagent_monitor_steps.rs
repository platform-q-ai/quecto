use super::*;
use quecto::infrastructure::tools::subagent_monitor::{apply_event, mark_exited};
use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentStatus};

// ===========================================================================
// Subagent Monitor BDD Steps (#522)
// ===========================================================================

// --- Given ---

#[given("a new SubagentEntry with status Starting")]
fn given_new_entry_starting(world: &mut QuectoWorld) {
    world.monitor_entry = Some(SubagentEntry::new(
        std::path::PathBuf::from("/stub/test.sock"),
        0,
    ));
}

#[given("subagent status variants Starting, Idle, Running, Error, Exited")]
fn given_all_status_variants(world: &mut QuectoWorld) {
    // Store all variants for display check in the Then step
    world.monitor_status_variants = Some(vec![
        SubagentStatus::Starting,
        SubagentStatus::Idle,
        SubagentStatus::Running,
        SubagentStatus::Error,
        SubagentStatus::Exited,
    ]);
}

#[given(expr = "a SubagentEntry with status {word}")]
fn given_entry_with_status(world: &mut QuectoWorld, status: String) {
    let s = match status.as_str() {
        "Starting" => SubagentStatus::Starting,
        "Idle" => SubagentStatus::Idle,
        "Running" => SubagentStatus::Running,
        "Error" => SubagentStatus::Error,
        "Exited" => SubagentStatus::Exited,
        _ => panic!("unknown status: {}", status),
    };
    let mut entry = SubagentEntry::new(std::path::PathBuf::from("/stub/test.sock"), 0);
    entry.status = s;
    world.monitor_entry = Some(entry);
}

#[given(expr = "a SubagentEntry with socket_path {string} and pid {int}")]
fn given_entry_with_socket_and_pid(world: &mut QuectoWorld, socket_path: String, pid: i32) {
    let entry = SubagentEntry::new(std::path::PathBuf::from(socket_path), pid as u32);
    world.monitor_entry = Some(entry);
}

// --- Grandchild propagation (#815) ---

#[given(
    expr = "a child's subagent_state_changed listing grandchild {string} under {string} and grandchild {string} under {string}"
)]
fn given_child_state_changed_with_two_grandchildren(
    world: &mut QuectoWorld,
    gc_a: String,
    parent_a: String,
    gc_b: String,
    parent_b: String,
) {
    // Build the full event JSON in the Given so the When is purely "forward".
    world.event_identity_last = Some(serde_json::json!({
        "type": "subagent_state_changed",
        "subagents": [
            { "agentId": gc_a, "status": "running", "parentId": parent_a, "pid": 0 },
            { "agentId": gc_b, "status": "idle", "parentId": parent_b, "pid": 0 },
        ],
    }));
}

#[when("the monitor forwards the child's subagent_state_changed event")]
fn when_monitor_forwards_state_changed(world: &mut QuectoWorld) {
    use quecto::infrastructure::tools::subagent_monitor::forward_child_state_changed;
    use quecto::infrastructure::tools::subagent_registry::new_registry;
    let line = world
        .event_identity_last
        .as_ref()
        .expect("no child event prepared")
        .to_string();
    // A fresh root registry: the forward merges the descendants in and emits the
    // union, so the forwarded event lists the grandchildren with real identity.
    let registry = new_registry();
    let forwarded = forward_child_state_changed(&line, &registry, "child")
        .expect("a subagent_state_changed line should be forwarded");
    world.event_identity_last = Some(serde_json::from_str(&forwarded).unwrap());
}

#[then(expr = "the forwarded event should list {string} with parent_id {string}")]
fn then_forwarded_lists_descendant(world: &mut QuectoWorld, grandchild: String, parent: String) {
    let ev = world
        .event_identity_last
        .as_ref()
        .expect("no forwarded event");
    assert_eq!(ev["type"].as_str(), Some("subagent_state_changed"));
    let arr = ev["subagents"].as_array().expect("subagents array");
    let entry = arr
        .iter()
        .find(|s| s["agentId"].as_str() == Some(grandchild.as_str()))
        .expect("forwarded event should contain the grandchild");
    assert_eq!(
        entry["parentId"].as_str(),
        Some(parent.as_str()),
        "grandchild must keep its real parent_id (not be re-stamped to the child)"
    );
}

#[given("a monitor abort handle")]
fn given_abort_handle(world: &mut QuectoWorld) {
    // Create a JoinHandle for a long-running task we can abort
    let rt = tokio::runtime::Runtime::new().unwrap();
    let handle = rt.spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    });
    world.monitor_abort_handle = Some(handle);
    world._monitor_rt = Some(rt);
}

// --- When ---

#[when(expr = "the monitor receives an {string} event")]
fn when_monitor_receives_event(world: &mut QuectoWorld, event_type: String) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    let event_json = match event_type.as_str() {
        "agent_start" => r#"{"type":"agent_start"}"#.to_string(),
        "agent_end" => r#"{"type":"agent_end","messages":[]}"#.to_string(),
        _ => panic!("unknown event type: {}", event_type),
    };
    apply_event(entry, &event_json);
}

#[when(expr = "the monitor receives a {string} event with tool_name {string}")]
fn when_monitor_receives_tool_start(
    world: &mut QuectoWorld,
    event_type: String,
    tool_name: String,
) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    let event_json = match event_type.as_str() {
        "tool_execution_start" => format!(
            r#"{{"type":"tool_execution_start","toolCallId":"c1","toolName":"{}","args":{{}}}}"#,
            tool_name
        ),
        _ => panic!("unexpected event type: {}", event_type),
    };
    apply_event(entry, &event_json);
}

#[when(expr = "the monitor receives a {string} event with is_error {word} and tool_name {string}")]
fn when_monitor_receives_tool_end(
    world: &mut QuectoWorld,
    _event_type: String,
    is_error: String,
    tool_name: String,
) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    let is_error_bool = is_error == "true";
    let event_json = format!(
        r#"{{"type":"tool_execution_end","toolCallId":"c1","toolName":"{}","result":{{"content":[]}},"isError":{}}}"#,
        tool_name, is_error_bool
    );
    apply_event(entry, &event_json);
}

#[when("the monitor detects connection closed")]
fn when_monitor_connection_closed(world: &mut QuectoWorld) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    mark_exited(entry);
}

#[when(expr = "apply_event is called with {string}")]
fn when_apply_event_with_json(world: &mut QuectoWorld, json: String) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    apply_event(entry, &json);
}

#[when("the abort handle is triggered")]
fn when_abort_handle_triggered(world: &mut QuectoWorld) {
    let handle = world.monitor_abort_handle.take().expect("no abort handle");
    handle.abort();
    let rt = world._monitor_rt.as_ref().expect("no runtime");
    let result = rt.block_on(handle);
    world.monitor_abort_result = Some(result.is_err()); // Should be JoinError (cancelled)
}

// --- Then ---

#[then(expr = "the subagent status should be {string}")]
fn then_subagent_status(world: &mut QuectoWorld, expected: String) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    let actual = format!("{}", entry.status);
    assert_eq!(
        actual, expected,
        "expected status '{}', got '{}'",
        expected, actual
    );
}

#[then("each variant should have a distinct display string")]
fn then_all_variants_distinct(world: &mut QuectoWorld) {
    let variants = world
        .monitor_status_variants
        .as_ref()
        .expect("no status variants");
    let displays: Vec<String> = variants.iter().map(|v| format!("{}", v)).collect();
    let unique: std::collections::HashSet<&String> = displays.iter().collect();
    assert_eq!(
        displays.len(),
        unique.len(),
        "expected all display strings to be unique, got: {:?}",
        displays
    );
}

#[then(expr = "the last_tool should be {string}")]
fn then_last_tool(world: &mut QuectoWorld, expected: String) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert_eq!(
        entry.last_tool.as_deref(),
        Some(expected.as_str()),
        "expected last_tool '{}', got: {:?}",
        expected,
        entry.last_tool
    );
}

#[then("the last_tool should be None")]
fn then_last_tool_none(world: &mut QuectoWorld) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert!(
        entry.last_tool.is_none(),
        "expected last_tool to be None, got: {:?}",
        entry.last_tool
    );
}

#[then(expr = "the last_error should contain {string}")]
fn then_last_error_contains(world: &mut QuectoWorld, expected: String) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    let err = entry.last_error.as_deref().expect("last_error is None");
    assert!(
        err.contains(&expected),
        "expected last_error to contain '{}', got: {}",
        expected,
        err
    );
}

#[then("the last_error should be None")]
fn then_last_error_none(world: &mut QuectoWorld) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert!(
        entry.last_error.is_none(),
        "expected last_error to be None, got: {:?}",
        entry.last_error
    );
}

#[then(expr = "the subagent entry should have socket_path {string}")]
fn then_entry_socket_path(world: &mut QuectoWorld, expected: String) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert_eq!(
        entry.socket_path.to_str().unwrap(),
        expected,
        "expected socket_path '{}'",
        expected
    );
}

#[then(expr = "the subagent entry should have pid {int}")]
fn then_entry_pid(world: &mut QuectoWorld, expected: i32) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert_eq!(entry.pid, expected as u32, "expected pid {}", expected);
}

#[then(expr = "the subagent registry entry {string} should have status {string}")]
fn then_registry_entry_status(world: &mut QuectoWorld, agent_id: String, expected: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let registry = tool.registry();
    let entries = registry.lock().unwrap();
    let entry = entries
        .iter()
        .find(|(key, entry)| key.as_str() == agent_id || entry.display_name == agent_id)
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| panic!("agent '{}' not found in registry", agent_id));
    let actual = format!("{}", entry.status);
    assert_eq!(
        actual, expected,
        "expected registry entry '{}' status '{}', got '{}'",
        agent_id, expected, actual
    );
}

#[then("the monitor task should be cancelled")]
fn then_monitor_cancelled(world: &mut QuectoWorld) {
    let was_err = world.monitor_abort_result.expect("no abort result");
    assert!(
        was_err,
        "expected JoinHandle to return JoinError (cancelled)"
    );
}

// --- Cascade-remove + broadcast on exit/kill (#831) ---

#[given(
    expr = "a root registry with parent {string}, child {string} under {string}, and grandchild {string} under {string}, plus a live agent {string}"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "cucumber step mirrors feature parameters"
)]
fn given_root_registry_tree(
    world: &mut QuectoWorld,
    parent: String,
    child: String,
    child_parent: String,
    grandchild: String,
    grandchild_parent: String,
    live: String,
) {
    use quecto::infrastructure::tools::subagent_registry::new_registry;
    let r = new_registry();
    {
        let mut g = r.lock().unwrap();
        g.insert(
            parent.clone(),
            SubagentEntry::new(std::path::PathBuf::from("/s"), 1),
        );
        let mut c = SubagentEntry::new(std::path::PathBuf::from("/s"), 2);
        c.parent_id = Some(child_parent);
        g.insert(child, c);
        let mut gc = SubagentEntry::new(std::path::PathBuf::from("/s"), 3);
        gc.parent_id = Some(grandchild_parent);
        g.insert(grandchild, gc);
        g.insert(live, SubagentEntry::new(std::path::PathBuf::from("/s"), 4));
    }
    world.cascade_registry = Some(r);
}

#[when(expr = "the parent {string} is killed")]
fn when_parent_exits_cascade(world: &mut QuectoWorld, parent: String) {
    use quecto::infrastructure::tools::subagent_cascade::cascade_remove_and_state_changed;
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    let event = cascade_remove_and_state_changed(r, &parent)
        .event
        .map(|s: String| serde_json::from_str::<serde_json::Value>(&s).unwrap());
    world.cascade_broadcast = Some(event);
}

#[when(expr = "an unknown agent {string} is reported gone")]
fn when_unknown_exits_cascade(world: &mut QuectoWorld, ghost: String) {
    when_parent_exits_cascade(world, ghost);
}

#[then(expr = "the broadcast subagent_state_changed should list only {string}")]
fn then_broadcast_lists_only(world: &mut QuectoWorld, only: String) {
    let ev = world
        .cascade_broadcast
        .as_ref()
        .expect("no cascade broadcast recorded")
        .as_ref()
        .expect("expected a broadcast event");
    assert_eq!(ev["type"].as_str(), Some("subagent_state_changed"));
    let ids: Vec<&str> = ev["subagents"]
        .as_array()
        .expect("subagents array")
        .iter()
        .filter_map(|s| s["agentId"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec![only.as_str()],
        "broadcast must list only survivors"
    );
}

#[then(expr = "the registry should no longer contain {string}, {string}, or {string}")]
fn then_registry_lacks_three(world: &mut QuectoWorld, a: String, b: String, c: String) {
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    let g = r.lock().unwrap();
    for id in [a.as_str(), b.as_str(), c.as_str()] {
        let entry = g
            .get(id)
            .unwrap_or_else(|| panic!("registry must retain tombstone for {id}"));
        assert_eq!(
            entry.status,
            quecto::infrastructure::tools::subagent_registry::SubagentStatus::Exited
        );
    }
}

#[then(expr = "the registry should still contain {string}")]
fn then_registry_contains(world: &mut QuectoWorld, id: String) {
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    assert!(
        r.lock().unwrap().contains_key(&id),
        "registry must contain {id}"
    );
}

// --- Scoped-replace prune of a forwarded subtree (#831 nested case) ---

#[given(
    expr = "a root registry with child {string} and a previously-merged grandchild {string} under it"
)]
fn given_root_with_merged_grandchild(world: &mut QuectoWorld, child: String, grandchild: String) {
    use quecto::infrastructure::tools::subagent_registry::new_registry;
    let r = new_registry();
    {
        let mut g = r.lock().unwrap();
        g.insert(
            child.clone(),
            SubagentEntry::new(std::path::PathBuf::from("/s"), 1),
        );
        let mut gc = SubagentEntry::new(std::path::PathBuf::from("/s"), 2);
        gc.parent_id = Some(child);
        g.insert(grandchild, gc);
    }
    world.cascade_registry = Some(r);
}

#[when(expr = "the child {string} forwards a subagent_state_changed with no descendants")]
fn when_child_forwards_empty(world: &mut QuectoWorld, child: String) {
    use quecto::infrastructure::tools::subagent_monitor::forward_child_state_changed;
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    let line = r#"{"type":"subagent_state_changed","subagents":[]}"#;
    let event = forward_child_state_changed(line, r, &child)
        .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap());
    world.cascade_broadcast = Some(event);
}

#[then(expr = "the forwarded event should not list {string}")]
fn then_forwarded_omits(world: &mut QuectoWorld, id: String) {
    let ev = world
        .cascade_broadcast
        .as_ref()
        .expect("no forwarded event recorded")
        .as_ref()
        .expect("expected a forwarded event");
    let listed = ev["subagents"]
        .as_array()
        .expect("subagents array")
        .iter()
        .any(|s| s["agentId"].as_str() == Some(id.as_str()));
    assert!(!listed, "forwarded event must not list pruned {id}");
}

#[then(expr = "the registry should no longer contain {string}")]
fn then_registry_lacks_one(world: &mut QuectoWorld, id: String) {
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    assert!(
        !r.lock().unwrap().contains_key(&id),
        "registry must not contain {id}"
    );
}

#[then("no subagent_state_changed broadcast is emitted")]
fn then_no_broadcast(world: &mut QuectoWorld) {
    let recorded = world
        .cascade_broadcast
        .as_ref()
        .expect("no cascade broadcast recorded");
    assert!(recorded.is_none(), "expected no broadcast event");
}

#[given(expr = "a root registry with child {string} and sibling {string}")]
fn given_root_registry_with_child_and_sibling(
    world: &mut QuectoWorld,
    child: String,
    sibling: String,
) {
    use quecto::infrastructure::tools::subagent_registry::new_registry;
    let r = new_registry();
    {
        let mut g = r.lock().unwrap();
        g.insert(child, SubagentEntry::new(std::path::PathBuf::from("/s"), 1));
        g.insert(
            sibling,
            SubagentEntry::new(std::path::PathBuf::from("/s"), 2),
        );
    }
    world.cascade_registry = Some(r);
}

#[given("an empty root subagent registry")]
fn given_empty_root_subagent_registry(world: &mut QuectoWorld) {
    use quecto::infrastructure::tools::subagent_registry::new_registry;
    world.cascade_registry = Some(new_registry());
}

#[when(expr = "child {string} forwards grandchild {string} as running")]
fn when_child_forwards_running_grandchild(
    world: &mut QuectoWorld,
    child: String,
    grandchild: String,
) {
    use quecto::infrastructure::tools::subagent_monitor::forward_child_state_changed;
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    let line = serde_json::json!({
        "type": "subagent_state_changed",
        "subagents": [{ "agentId": grandchild, "status": "running", "parentId": child }],
    })
    .to_string();
    let event = forward_child_state_changed(&line, r, &child)
        .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap());
    world.event_identity_last = event.clone();
    world.cascade_broadcast = Some(event);
}

#[when(expr = "child {string} forwards {int} running grandchildren")]
fn when_child_forwards_n_running_grandchildren(
    world: &mut QuectoWorld,
    child: String,
    count: usize,
) {
    use quecto::infrastructure::tools::subagent_monitor::forward_child_state_changed;
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    let subagents: Vec<serde_json::Value> = (0..count)
        .map(|i| {
            serde_json::json!({
                "agentId": format!("gc-{i}"),
                "status": "running",
                "parentId": child,
            })
        })
        .collect();
    let line = serde_json::json!({
        "type": "subagent_state_changed",
        "subagents": subagents,
    })
    .to_string();
    let event = forward_child_state_changed(&line, r, &child)
        .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap());
    world.cascade_broadcast = Some(event);
}

#[then(expr = "the forwarded event should list {string}")]
fn then_forwarded_lists_agent(world: &mut QuectoWorld, id: String) {
    let ev = world
        .cascade_broadcast
        .as_ref()
        .expect("no forwarded event recorded")
        .as_ref()
        .expect("expected a forwarded event");
    let listed = ev["subagents"]
        .as_array()
        .expect("subagents array")
        .iter()
        .any(|s| s["agentId"].as_str() == Some(id.as_str()));
    assert!(listed, "forwarded event must list {id}: {ev}");
}

#[then(expr = "the registry should contain {int} subagents")]
fn then_registry_contains_n_subagents(world: &mut QuectoWorld, expected: usize) {
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    assert_eq!(
        r.lock().unwrap().len(),
        expected,
        "registry should cap merged descendants at {expected}"
    );
}

#[then(expr = "the forwarded event should contain {int} subagents")]
fn then_forwarded_event_contains_n_subagents(world: &mut QuectoWorld, expected: usize) {
    let ev = world
        .cascade_broadcast
        .as_ref()
        .expect("no forwarded event recorded")
        .as_ref()
        .expect("expected a forwarded event");
    let actual = ev["subagents"].as_array().expect("subagents array").len();
    assert_eq!(
        actual, expected,
        "forwarded union should contain exactly the capped registry set"
    );
}

// --- Prompt idle propagation for nested agents (#839) ---

#[given(expr = "a root monitor knows grandchild {string} under {string} is idle")]
fn given_root_monitor_knows_idle_grandchild(
    world: &mut QuectoWorld,
    grandchild: String,
    child: String,
) {
    use quecto::infrastructure::tools::subagent_registry::new_registry;
    let r = new_registry();
    {
        let mut g = r.lock().unwrap();
        g.insert(
            child.clone(),
            SubagentEntry::new(std::path::PathBuf::from("/s"), 1),
        );
        let mut gc = SubagentEntry::new(std::path::PathBuf::from("/s"), 2);
        gc.parent_id = Some(child);
        gc.status = SubagentStatus::Idle;
        g.insert(grandchild, gc);
    }
    world.cascade_registry = Some(r);
}

#[when(expr = "the child reports grandchild {string} without a status update")]
fn when_child_reports_grandchild_without_status(world: &mut QuectoWorld, grandchild: String) {
    use quecto::infrastructure::tools::subagent_monitor::forward_child_state_changed;
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    let parent = r
        .lock()
        .unwrap()
        .get(&grandchild)
        .and_then(|e| e.parent_id.clone())
        .expect("grandchild must have parent");
    let line = serde_json::json!({
        "type": "subagent_state_changed",
        "subagents": [{ "agentId": grandchild, "parentId": parent }],
    })
    .to_string();
    let event = forward_child_state_changed(&line, r, &parent)
        .map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap());
    world.cascade_broadcast = Some(event);
}

#[then(expr = "the monitor should keep {string} idle")]
fn then_monitor_keeps_idle(world: &mut QuectoWorld, agent_id: String) {
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    assert_eq!(
        r.lock().unwrap()[&agent_id].status,
        SubagentStatus::Idle,
        "monitor must preserve the known idle status"
    );
}

#[then(expr = "observers should see {string} as idle")]
fn then_observers_see_idle(world: &mut QuectoWorld, agent_id: String) {
    let ev = world
        .cascade_broadcast
        .as_ref()
        .expect("no state event recorded")
        .as_ref()
        .expect("expected a state event");
    let entry = ev["subagents"]
        .as_array()
        .expect("subagents array")
        .iter()
        .find(|s| s["agentId"].as_str() == Some(agent_id.as_str()))
        .expect("agent should be listed for observers");
    assert_eq!(entry["status"].as_str(), Some("idle"));
}

#[given(expr = "a root monitor with running child {string}")]
fn given_root_monitor_with_running_child(world: &mut QuectoWorld, child: String) {
    use quecto::infrastructure::tools::subagent_registry::new_registry;
    let r = new_registry();
    let mut entry = SubagentEntry::new(std::path::PathBuf::from("/s"), 1);
    entry.status = SubagentStatus::Running;
    r.lock().unwrap().insert(child, entry);
    world.cascade_registry = Some(r);
}

#[when(expr = "the child {string} finishes its turn")]
fn when_child_finishes_turn(world: &mut QuectoWorld, child: String) {
    use quecto::infrastructure::tools::subagent_cascade::build_state_changed_event;
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    {
        let mut g = r.lock().unwrap();
        let entry = g
            .get_mut(&child)
            .unwrap_or_else(|| panic!("child {child} not found"));
        quecto::infrastructure::tools::subagent_monitor::apply_event(
            entry,
            r#"{"type":"agent_end","messages":[]}"#,
        );
    }
    let event = serde_json::from_str::<serde_json::Value>(&build_state_changed_event(r)).unwrap();
    world.cascade_broadcast = Some(Some(event));
}

#[then(expr = "observers should receive subagent state listing {string} as idle")]
fn then_observers_receive_child_idle(world: &mut QuectoWorld, child: String) {
    let ev = world
        .cascade_broadcast
        .as_ref()
        .expect("no state event recorded")
        .as_ref()
        .expect("expected a state event");
    let entry = ev["subagents"]
        .as_array()
        .expect("subagents array")
        .iter()
        .find(|s| s["agentId"].as_str() == Some(child.as_str()))
        .expect("child should be listed for observers");
    assert_eq!(entry["status"].as_str(), Some("idle"));
}

#[given(expr = "a forwarded subagent state listing idle grandchild {string} under {string}")]
fn given_forwarded_state_with_idle_grandchild(
    world: &mut QuectoWorld,
    grandchild: String,
    child: String,
) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut h = rt.block_on(quecto_tui::shell::app::tui_harness::TuiHarness::new());
    let mut info = quecto_tui::shell::app::tui_harness::subagent(&grandchild, "idle", None);
    info.parent_id = Some(child);
    h.event(quecto_tui::shell::app::tui_harness::subagents_changed(
        vec![info],
    ));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

#[when("the TUI renders the forwarded subagent state")]
fn when_tui_renders_forwarded_state(world: &mut QuectoWorld) {
    let h = &mut world.tui_parity.as_mut().expect("no TUI harness").0;
    let frame = h.full_frame();
    // The panel has no header now; prove it rendered its chrome via the footer
    // hint (panel-specific, not emitted by the main pane).
    assert!(
        frame.contains("pane"),
        "rendered frame should include the subagent panel footer:\n{frame}"
    );
    world.stdout = frame;
}

#[then(expr = "the subagent panel should show {string} as idle")]
fn then_panel_shows_idle(world: &mut QuectoWorld, agent_id: String) {
    // Idle status is carried by the name COLOUR (yellow), not a word. Verify the
    // agent is present AND its row carries the yellow (idle) code and not green
    // (running). The grandchild scenario is minimal, so the main-pane portion of
    // the joined row doesn't introduce these colours.
    let h = &mut world.tui_parity.as_mut().expect("no TUI harness").0;
    assert!(
        h.full_frame().contains(&agent_id),
        "subagent panel should show {agent_id}"
    );
    let raw = h.full_frame_raw();
    let row = raw
        .lines()
        .find(|l| l.contains(&agent_id))
        .unwrap_or_else(|| panic!("{agent_id} row not found"))
        .to_string();
    assert!(
        row.contains("\x1b[33m") && !row.contains("\x1b[32m"),
        "idle {agent_id} must render yellow (idle), not green (running): {row:?}"
    );
}

// --- Prompt running visibility on a long first turn (#866) ---

#[when(expr = "the child {string} starts its turn")]
fn when_child_starts_turn(world: &mut QuectoWorld, child: String) {
    use quecto::infrastructure::tools::subagent_cascade::build_state_changed_event;
    use quecto::infrastructure::tools::subagent_monitor::should_broadcast_state_changed_after_event;
    let r = world
        .cascade_registry
        .as_ref()
        .expect("no cascade registry");
    let value = serde_json::json!({"type": "agent_start"});
    {
        let mut g = r.lock().unwrap();
        let entry = g
            .get_mut(&child)
            .unwrap_or_else(|| panic!("child {child} not found"));
        quecto::infrastructure::tools::subagent_monitor::apply_event_parsed(entry, &value);
    }
    // Drive the real broadcast-trigger gate (#839/#866): only record a broadcast
    // if `agent_start` actually qualifies. Initialise to `Some(None)` so the Then
    // assertion fails if the gate ever suppresses the running transition.
    world.cascade_broadcast = Some(None);
    if should_broadcast_state_changed_after_event(&value) {
        let event =
            serde_json::from_str::<serde_json::Value>(&build_state_changed_event(r)).unwrap();
        world.cascade_broadcast = Some(Some(event));
    }
}

#[then(expr = "observers should receive subagent state listing {string} as running")]
fn then_observers_receive_child_running(world: &mut QuectoWorld, child: String) {
    let ev = world
        .cascade_broadcast
        .as_ref()
        .expect("no state event recorded")
        .as_ref()
        .expect("expected a state event");
    let entry = ev["subagents"]
        .as_array()
        .expect("subagents array")
        .iter()
        .find(|s| s["agentId"].as_str() == Some(child.as_str()))
        .expect("child should be listed for observers");
    assert_eq!(entry["status"].as_str(), Some("running"));
}

#[given(expr = "the TUI is tracking a spawning agent {string}")]
fn given_tui_tracking_spawning_agent(world: &mut QuectoWorld, agent_id: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut h = rt.block_on(quecto_tui::shell::app::tui_harness::TuiHarness::new());
    // The spawn ToolStart registers the child locally as "starting" before the
    // kernel has confirmed it (the #866 pre-registration window).
    h.event(quecto_tui::shell::app::tui_harness::spawn_start(&agent_id));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

#[when(expr = "a subagent push arrives that omits {string}")]
fn when_push_omits_agent(world: &mut QuectoWorld, agent_id: String) {
    let h = &mut world.tui_parity.as_mut().expect("no TUI harness").0;
    let other = format!("not-{agent_id}");
    h.event(quecto_tui::shell::app::tui_harness::subagents_changed(
        vec![quecto_tui::shell::app::tui_harness::subagent(
            &other, "running", None,
        )],
    ));
}

#[then(expr = "the subagent panel should still show {string}")]
fn then_panel_still_shows(world: &mut QuectoWorld, agent_id: String) {
    let h = &mut world.tui_parity.as_mut().expect("no TUI harness").0;
    let frame = h.full_frame();
    assert!(
        frame.contains(&agent_id),
        "#866: an unconfirmed spawning agent must stay visible after an omitting push:\n{frame}"
    );
}

#[then(expr = "the subagent panel should not count {string} as working")]
fn then_panel_does_not_count_working(world: &mut QuectoWorld, agent_id: String) {
    let h = &mut world.tui_parity.as_mut().expect("no TUI harness").0;
    let bottom = h.bottom_stack();
    assert!(
        !bottom.contains("working"),
        "idle {agent_id} must not count as working:\n{bottom}"
    );
}
