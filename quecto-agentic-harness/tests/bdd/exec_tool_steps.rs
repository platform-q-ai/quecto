// BDD step definitions for ExecTool Quecto-compatible scenarios (issue #146).

use std::sync::Arc;

use cucumber::{given, when};

use super::*;

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
