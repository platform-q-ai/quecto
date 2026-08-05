use super::*;
use quecto::interface::cli::protocol::{AgentEvent, SubagentInfo};

// ─── SubagentBar widget integration steps (#525) ──────────────────────────────
// These test the protocol data that feeds the TUI widget, not the widget itself.
// The actual widget rendering is tested in quecto-tui unit tests.

#[given("a SubagentBar with no agents")]
fn given_empty_bar(world: &mut QuectoWorld) {
    world.widget_subagent_infos = vec![];
}

#[given("a SubagentBar with agents:")]
fn given_bar_with_agents(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    let table = step.table.as_ref().expect("expected a data table");
    let agents: Vec<SubagentInfo> = table
        .rows
        .iter()
        .skip(1)
        .map(|row| SubagentInfo {
            agent_uuid: None,
            display_name: None,
            agent_id: row[0].clone(),
            status: row[1].clone(),
            last_tool: if row[2].is_empty() {
                None
            } else {
                Some(row[2].clone())
            },
            last_error: if row[3].is_empty() {
                None
            } else {
                Some(row[3].clone())
            },
            pid: 0,
            socket_path: None,
            parent_id: None,
            workflow: None,
            read_only: false,
            runtime_backend: "local".to_string(),
            container_uuid: None,
            container_ref: None,
            container_name: None,
            repo_url: None,
            environment_id: None,
            environment_health: None,
            socket_mode: None,
            workspace_path: None,
        })
        .collect();
    world.widget_subagent_infos = agents;
}

#[when(expr = "I render the bar at width {int}")]
fn when_render_bar(world: &mut QuectoWorld, _width: usize) {
    // Simulate what the TUI does: serialize the SubagentStateChanged event,
    // then verify the data is correctly shaped.
    let ev = AgentEvent::SubagentStateChanged {
        subagents: world.widget_subagent_infos.clone(),
    };
    let json = ev.to_json_line();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let subagents = parsed["subagents"].as_array().unwrap();
    // Each subagent produces one "line" in the widget
    world.widget_bar_lines = subagents
        .iter()
        .map(|s| {
            let id = s["agentId"].as_str().unwrap_or("");
            let status = s["status"].as_str().unwrap_or("");
            let tool = s.get("lastTool").and_then(|v| v.as_str()).unwrap_or("");
            let error = s.get("lastError").and_then(|v| v.as_str()).unwrap_or("");
            let status_label = match status {
                "running" => "Running",
                "idle" => "Idle",
                "error" => "Error",
                "starting" => "Starting",
                "exited" => "Exited",
                _ => status,
            };
            let mut line = format!("  {} [bar] {}", id, status_label);
            if !tool.is_empty() {
                line.push_str(&format!(" · {}", tool));
            }
            if !error.is_empty() {
                line.push_str(&format!(" · {}", error));
            }
            line
        })
        .collect();
}

#[when("I update the bar with an empty list")]
fn when_update_empty(world: &mut QuectoWorld) {
    world.widget_subagent_infos = vec![];
}

#[then("the rendered output should be empty")]
fn then_output_empty(world: &mut QuectoWorld) {
    assert!(
        world.widget_bar_lines.is_empty(),
        "expected empty output, got {} lines",
        world.widget_bar_lines.len()
    );
}

#[then(expr = "the rendered output should have {int} lines")]
fn then_output_lines(world: &mut QuectoWorld, count: usize) {
    assert_eq!(world.widget_bar_lines.len(), count);
}

#[then(expr = "the first line should contain {string}")]
fn then_first_line_contains(world: &mut QuectoWorld, text: String) {
    let line = &world.widget_bar_lines[0];
    assert!(
        line.contains(&text),
        "first line missing '{}': {}",
        text,
        line
    );
}

#[then(expr = "the second line should contain {string}")]
fn then_second_line_contains(world: &mut QuectoWorld, text: String) {
    let line = &world.widget_bar_lines[1];
    assert!(
        line.contains(&text),
        "second line missing '{}': {}",
        text,
        line
    );
}
