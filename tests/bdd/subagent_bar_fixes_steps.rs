use super::*;
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};
use quecto::interface::cli::protocol::build_subagent_info_list;
use std::path::PathBuf;

// ─── Fix 1: Render order (compile-time verified) ─────────────────────────────

#[given("a render bottom section layout")]
fn given_render_bottom(world: &mut QuectoWorld) {
    // No-op: the render order is verified by the TUI unit tests and code structure.
    world.stdout = String::new();
}

#[then("widgets_above should come before spinner in the output order")]
fn then_widgets_above_before_spinner(_world: &mut QuectoWorld) {
    // Verify the render order by reading the source file and checking
    // that widgets_above.render() appears before spinner.render() in
    // the bottom section.
    let source = std::fs::read_to_string("quecto-tui/src/interface/app.rs")
        .expect("should be able to read interface/app.rs");
    let widgets_pos = source
        .find("widgets_above.render(width)")
        .expect("widgets_above.render not found");
    let spinner_pos = source
        .find("spinner.render(width)")
        .expect("spinner.render not found");
    assert!(
        widgets_pos < spinner_pos,
        "widgets_above.render() (at byte {}) should appear before spinner.render() (at byte {})",
        widgets_pos,
        spinner_pos
    );
}

// ─── Fix 2 & 3: SubagentStateChanged from registry ──────────────────────────

#[given(expr = "a subagent registry with agent {string} status {string}")]
fn given_registry_with_agent(world: &mut QuectoWorld, agent_id: String, status: String) {
    let registry = new_registry();
    {
        let mut guard = registry.lock().unwrap();
        let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 42);
        entry.status = match status.as_str() {
            "running" => SubagentStatus::Running,
            "idle" => SubagentStatus::Idle,
            "error" => SubagentStatus::Error,
            "exited" => SubagentStatus::Exited,
            "starting" => SubagentStatus::Starting,
            _ => SubagentStatus::Starting,
        };
        guard.insert(agent_id, entry);
    }
    world.subagent_protocol_registry = Some(registry);
}

#[when("I build the subagent info list from the registry")]
fn when_build_info_list(world: &mut QuectoWorld) {
    let list = build_subagent_info_list(&world.subagent_protocol_registry);
    world.subagent_infos = list;
}

#[then(expr = "the list should contain {int} entry")]
fn then_list_contains_n(world: &mut QuectoWorld, n: usize) {
    assert_eq!(
        world.subagent_infos.len(),
        n,
        "expected {} entries, got {}",
        n,
        world.subagent_infos.len()
    );
}

#[then(expr = "the entry agent_id should be {string}")]
fn then_entry_agent_id(world: &mut QuectoWorld, expected: String) {
    assert_eq!(world.subagent_infos[0].agent_id, expected);
}

#[then(expr = "the entry status should be {string}")]
fn then_entry_status(world: &mut QuectoWorld, expected: String) {
    assert_eq!(world.subagent_infos[0].status, expected);
}
