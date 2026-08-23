// BDD step definitions for ExecTool Quecto-compatible scenarios (issue #146).

use std::sync::Arc;

use cucumber::{given, then, when};

use super::*;
use crate::agent_tools_steps::interpret_escapes;

/// Execute bash with a SHELL env variable override via execute_with_env.
#[when(regex = r#"^the agent executes bash "([^"]+)" with shell env "([^"]+)"$"#)]
fn when_exec_bash_with_shell(world: &mut QuectoWorld, command: String, shell: String) {
    use quecto::infrastructure::tools::bash::{ExecOptions, ExecTool};
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let sandbox = Arc::new(quecto::infrastructure::security::sandbox::Sandbox::new(
        Some(ws.clone()),
    ));
    let tool = ExecTool::with_options(Arc::new(ws.clone()), sandbox, ExecOptions::default());

    let mut env_overrides = std::collections::HashMap::new();
    env_overrides.insert("SHELL".to_string(), format!("/bin/{}", shell));

    let args = serde_json::json!({"command": command}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute_with_env(&args, &env_overrides));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

/// Execute bash with a command prefix option (constructs a custom ExecTool).
#[when(regex = r#"^the agent executes bash with command prefix "([^"]+)" and command "([^"]+)"$"#)]
fn when_exec_bash_with_prefix(world: &mut QuectoWorld, prefix: String, command: String) {
    use quecto::infrastructure::tools::bash::{ExecOptions, ExecTool};
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let sandbox = Arc::new(quecto::infrastructure::security::sandbox::Sandbox::new(
        Some(ws.clone()),
    ));
    let opts = ExecOptions {
        command_prefix: Some(prefix),
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(ws.clone()), sandbox, opts);
    let args = serde_json::json!({"command": command}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(&args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

/// Generate a command that produces approximately N bytes of output (multi-line).
#[given(regex = r#"^a large output command that produces (\d+) bytes$"#)]
fn given_large_byte_output_command(world: &mut QuectoWorld, n: usize) {
    // Each printf line is ~50 bytes; repeat to exceed n total.
    let lines = (n / 50).max(1);
    world.stored_command = Some(format!(
        "for i in $(seq 1 {}); do printf 'line%04d: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\n' $i; done",
        lines
    ));
}

/// Generate a command that produces N lines of output.
#[given(regex = r#"^a large output command that produces (\d+) lines$"#)]
fn given_large_line_output_command(world: &mut QuectoWorld, n: usize) {
    world.stored_command = Some(format!("seq 1 {}", n));
}

/// Execute the previously stored command via the bash tool.
#[when("the agent executes that command via the bash tool")]
fn when_exec_stored_command(world: &mut QuectoWorld) {
    let cmd = world
        .stored_command
        .clone()
        .expect("no stored command (use Given to set one)");
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let args = serde_json::json!({"command": cmd}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("bash", &args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[when(
    regex = r#"^the agent executes bash with output_file \"([^\"]+)\" and command \"([\s\S]*)\"$"#
)]
fn when_exec_bash_with_output_file(world: &mut QuectoWorld, output_file: String, command: String) {
    execute_bash_with_optional_output_file(world, command, None, Some(output_file));
}

#[when(regex = r#"^the agent executes bash with timeout (\d+) and command \"([\s\S]*)\"$"#)]
fn when_exec_bash_with_timeout(world: &mut QuectoWorld, timeout: u64, command: String) {
    execute_bash_with_optional_output_file(world, command, Some(timeout), None);
}

#[when(
    regex = r#"^the agent executes bash with timeout (\d+) output_file \"([^\"]+)\" and command \"([\s\S]*)\"$"#
)]
fn when_exec_bash_with_timeout_and_output_file(
    world: &mut QuectoWorld,
    timeout: u64,
    output_file: String,
    command: String,
) {
    execute_bash_with_optional_output_file(world, command, Some(timeout), Some(output_file));
}

fn execute_bash_with_optional_output_file(
    world: &mut QuectoWorld,
    command: String,
    timeout: Option<u64>,
    output_file: Option<String>,
) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let command = interpret_escapes(&command);
    let mut args = serde_json::json!({"command": command});
    if let Some(timeout) = timeout {
        args["timeout"] = serde_json::json!(timeout);
    }
    if let Some(output_file) = output_file {
        args["output_file"] = serde_json::json!(output_file);
    }
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("bash", &args.to_string()));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the tool result should be shorter than {int} characters")]
fn then_tool_result_shorter_than(world: &mut QuectoWorld, max_len: usize) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            tr.content.len() < max_len,
            "expected tool result shorter than {}, got {}: {}",
            max_len,
            tr.content.len(),
            tr.content
        ),
        Err(e) => panic!("tool returned error: {}", e),
    }
}

#[then(expr = "bash output_file {string} should contain {string}")]
fn then_bash_output_file_contains(world: &mut QuectoWorld, output_file: String, expected: String) {
    let expected = interpret_escapes(&expected);
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let content = std::fs::read_to_string(ws.join(output_file)).expect("output_file should exist");
    assert_eq!(content, expected);
}

#[then(expr = "bash output_file {string} should contain {int} {string} characters")]
fn then_bash_output_file_repeated_characters(
    world: &mut QuectoWorld,
    output_file: String,
    count: usize,
    value: String,
) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let content = std::fs::read_to_string(ws.join(output_file)).expect("output_file should exist");
    assert_eq!(content.trim(), value.repeat(count));
}

#[then(expr = "the tool call should fail with {string}")]
fn then_tool_call_should_fail_with(world: &mut QuectoWorld, expected: String) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            tr.is_error && tr.content.contains(&expected),
            "expected tool error containing '{}', got: {}",
            expected,
            tr.content
        ),
        Err(e) => assert!(
            e.contains(&expected),
            "expected domain error containing '{}', got: {}",
            expected,
            e
        ),
    }
}
