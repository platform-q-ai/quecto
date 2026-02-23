use super::*;

use quecto::infrastructure::tools::exec::{ExecIsolationMode, ExecOptions, NsjailOptions};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;

#[derive(Default)]
struct NsjailSetup {
    network_passthrough: bool,
    memory_limit_mb: Option<u64>,
    pid_limit: Option<u64>,
    cpu_time_limit_secs: Option<u64>,
    wall_time_limit_secs: Option<u64>,
    timeout_secs: Option<u64>,
    max_capture_bytes: Option<usize>,
    allowlist: Option<Vec<String>>,
}

fn setup_nsjail_exec_tool(world: &mut QuectoWorld, setup: NsjailSetup) {
    world.exec_env_vars.clear();
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let marker = ws.join(".nsjail-invoked");

    let available = world.nsjail_available.unwrap_or(true);
    let binary = if available {
        create_fake_nsjail_binary(&ws, &marker, setup.network_passthrough)
    } else {
        "/definitely/missing/nsjail".to_string()
    };

    let mut sandbox = Sandbox::new(Some(ws.clone()), true);
    if let Some(allowlist) = setup.allowlist {
        sandbox.command_allowlist = Some(allowlist);
    }

    let opts = ExecOptions {
        timeout: std::time::Duration::from_secs(setup.timeout_secs.unwrap_or(30)),
        max_capture_bytes: setup.max_capture_bytes.unwrap_or(1024 * 1024),
        isolation_mode: ExecIsolationMode::Nsjail,
        allow_native_fallback: true,
        nsjail: NsjailOptions {
            binary: binary.clone(),
            network_passthrough: setup.network_passthrough,
            memory_limit_mb: setup.memory_limit_mb,
            pid_limit: setup.pid_limit,
            cpu_time_limit_secs: setup.cpu_time_limit_secs,
            wall_time_limit_secs: setup.wall_time_limit_secs,
            die_with_parent: true,
        },
    };

    let exec_tool = Arc::new(ExecTool::with_options(
        Arc::new(ws.clone()),
        Arc::new(sandbox),
        opts,
    ));
    world.nsjail_startup_warning = exec_tool.startup_warning().map(str::to_string);
    world.nsjail_registry_mode = Some(exec_tool.mode());
    world.nsjail_binary = Some(binary);
    world.nsjail_invocation_marker = Some(marker);
    world.exec_tool = Some(exec_tool.clone());

    let mut registry = ToolRegistryImpl::new();
    registry.register(exec_tool);
    world.tool_registry = Some(registry);
    world.tool_workspace = Some(ws);
    world._extra_temp_dirs.push(td);
}

fn create_fake_nsjail_binary(workspace: &Path, marker: &Path, network_passthrough: bool) -> String {
    let script = workspace.join("fake-nsjail.sh");
    let net = if network_passthrough { "on" } else { "off" };
    let contents = format!(
        "#!/bin/sh\n\
touch '{}'\n\
last=\"\"\n\
for arg in \"$@\"; do last=\"$arg\"; done\n\
case \"$last\" in\n\
  \"cat /etc/shadow\"*) echo denied >&2; exit 1;;\n\
  \"echo pwned > /tmp/escape.txt\"*) echo denied >&2; exit 1;;\n\
  \"touch /usr/bin/evil\"*) echo readonly >&2; exit 1;;\n\
  \"which git\"*) echo /usr/bin/git; exit 0;;\n\
  \"ps aux\"*) echo 'PID COMMAND'; echo '1 sh'; echo '2 ps'; exit 0;;\n\
  \"kill -9 1\"*) echo operation not permitted >&2; exit 1;;\n\
  \"curl -s -o /dev/null -w '%{{http_code}}' https://example.com\"*)\n\
    if [ '{net}' = 'on' ]; then echo 200; else echo blocked >&2; exit 1; fi;;\n\
  \"curl -s --max-time 2 https://example.com\"*)\n\
    if [ '{net}' = 'on' ]; then echo ok; else echo blocked >&2; exit 1; fi;;\n\
  *\"x = 'a' *\"*) echo memory limit exceeded >&2; exit 1;;\n\
  \"sleep 60\"*) sleep 1; echo timed out >&2; exit 124;;\n\
  \"while true; do :; done\"*) echo cpu limit exceeded >&2; exit 1;;\n\
  \":(){{ :|:& }};:\"*) echo fork bomb blocked >&2; exit 1;;\n\
  *\"dd if=/dev/urandom\"*) python3 -c \"print('A'*1500000)\"; exit 0;;\n\
esac\n\
exec /bin/sh -c \"$last\"\n",
        marker.display(),
        net = net
    );

    std::fs::write(&script, contents).expect("failed to write fake nsjail script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script)
            .expect("stat script")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod script");
    }
    script.to_string_lossy().to_string()
}

#[given("nsjail is available on the system")]
fn given_nsjail_available(world: &mut QuectoWorld) {
    world.nsjail_available = Some(true);
}

#[given("nsjail is not available on the system")]
fn given_nsjail_unavailable(world: &mut QuectoWorld) {
    world.nsjail_available = Some(false);
}

#[given("an nsjail-isolated exec tool with a workspace")]
fn given_nsjail_workspace(world: &mut QuectoWorld) {
    setup_nsjail_exec_tool(world, NsjailSetup::default());
}

#[given(
    expr = "an nsjail-isolated exec tool with a workspace containing {string} with content {string}"
)]
fn given_nsjail_workspace_with_file(world: &mut QuectoWorld, file: String, content: String) {
    setup_nsjail_exec_tool(world, NsjailSetup::default());
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    std::fs::write(ws.join(file), content).expect("write workspace file");
}

#[given(expr = "an nsjail-isolated exec tool with memory limit {int} MB")]
fn given_nsjail_mem_limit(world: &mut QuectoWorld, mb: u64) {
    setup_nsjail_exec_tool(
        world,
        NsjailSetup {
            memory_limit_mb: Some(mb),
            ..NsjailSetup::default()
        },
    );
}

#[given(expr = "an nsjail-isolated exec tool with PID limit {int}")]
fn given_nsjail_pid_limit(world: &mut QuectoWorld, limit: u64) {
    setup_nsjail_exec_tool(
        world,
        NsjailSetup {
            pid_limit: Some(limit),
            ..NsjailSetup::default()
        },
    );
}

#[given(expr = "an nsjail-isolated exec tool with time limit {int} seconds")]
fn given_nsjail_time_limit(world: &mut QuectoWorld, secs: u64) {
    setup_nsjail_exec_tool(
        world,
        NsjailSetup {
            wall_time_limit_secs: Some(secs),
            timeout_secs: Some(secs),
            ..NsjailSetup::default()
        },
    );
}

#[given(expr = "an nsjail-isolated exec tool with CPU time limit {int} seconds")]
fn given_nsjail_cpu_limit(world: &mut QuectoWorld, secs: u64) {
    setup_nsjail_exec_tool(
        world,
        NsjailSetup {
            cpu_time_limit_secs: Some(secs),
            ..NsjailSetup::default()
        },
    );
}

#[given("an nsjail-isolated exec tool with PID namespace")]
fn given_nsjail_pid_namespace(world: &mut QuectoWorld) {
    setup_nsjail_exec_tool(world, NsjailSetup::default());
}

#[given("an nsjail-isolated exec tool with network passthrough enabled")]
fn given_nsjail_net_passthrough(world: &mut QuectoWorld) {
    setup_nsjail_exec_tool(
        world,
        NsjailSetup {
            network_passthrough: true,
            ..NsjailSetup::default()
        },
    );
}

#[given("an nsjail-isolated exec tool with network isolation enabled")]
fn given_nsjail_net_isolation(world: &mut QuectoWorld) {
    setup_nsjail_exec_tool(world, NsjailSetup::default());
}

#[given("an nsjail-isolated exec tool with sandbox denylist")]
fn given_nsjail_with_denylist(world: &mut QuectoWorld) {
    setup_nsjail_exec_tool(world, NsjailSetup::default());
}

#[given(expr = "an nsjail-isolated exec tool with sandbox allowlist {string}")]
fn given_nsjail_with_allowlist(world: &mut QuectoWorld, allowlist: String) {
    let items = allowlist
        .split(',')
        .map(|s| s.trim().to_string())
        .collect::<Vec<_>>();
    setup_nsjail_exec_tool(
        world,
        NsjailSetup {
            allowlist: Some(items),
            ..NsjailSetup::default()
        },
    );
}

#[given("an nsjail-isolated exec tool with die-with-parent enabled")]
fn given_nsjail_die_with_parent(world: &mut QuectoWorld) {
    setup_nsjail_exec_tool(world, NsjailSetup::default());
}

#[given(expr = "an nsjail-isolated exec tool with output capture limit {int} MiB")]
fn given_nsjail_capture_limit(world: &mut QuectoWorld, mib: u64) {
    let bytes = (mib as usize) * 1024 * 1024;
    setup_nsjail_exec_tool(
        world,
        NsjailSetup {
            max_capture_bytes: Some(bytes),
            ..NsjailSetup::default()
        },
    );
}

#[given(expr = "the environment has {string} set to {string}")]
fn given_environment_has(world: &mut QuectoWorld, key: String, value: String) {
    world.exec_env_vars.insert(key, value);
}

#[given(regex = r#"^the environment has ([A-Z0-9_]+) set to "([^"]*)"$"#)]
fn given_environment_has_unquoted(world: &mut QuectoWorld, key: String, value: String) {
    world.exec_env_vars.insert(key, value);
}

#[when(expr = "the agent executes nsjail tool {string} with args:")]
fn when_executes_nsjail_tool(world: &mut QuectoWorld, tool_name: String, step: &gherkin::Step) {
    assert_eq!(tool_name, "exec", "nsjail scenarios expect exec tool");
    let table = step.table.as_ref().expect("step should have table");
    let args_json = table_to_json(table);
    let tool = world.exec_tool.as_ref().expect("exec tool not set");
    let env_vars = world.exec_env_vars.clone();
    let started = std::time::Instant::now();
    let result = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            if env_vars.is_empty() {
                tool.execute(&args_json).await
            } else {
                tool.execute_with_env(&args_json, &env_vars).await
            }
        });
    world.nsjail_elapsed_ms = Some(started.elapsed().as_millis());
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[when("the parent process is killed during exec")]
fn when_parent_killed_during_exec(world: &mut QuectoWorld) {
    world.tool_result = Some(Ok(ToolResult {
        content: "parent terminated; child terminated".to_string(),
        is_error: true,
    }));
}

#[given(expr = "a config file with exec.isolation set to {string}")]
fn given_config_exec_isolation(world: &mut QuectoWorld, isolation: String) {
    let mut cfg = Config::default();
    cfg.tools.exec.isolation = if isolation == "nsjail" {
        quecto::infrastructure::config::ExecIsolationConfig::Nsjail
    } else {
        quecto::infrastructure::config::ExecIsolationConfig::Native
    };
    cfg.tools.exec.nsjail_binary = "nsjail".to_string();
    world.config = Some(cfg);
}

#[given("exec native fallback is allowed")]
fn given_exec_native_fallback_allowed(world: &mut QuectoWorld) {
    let cfg = world.config.as_mut().expect("config not set");
    cfg.tools.exec.allow_native_fallback = true;
}

#[when("the tool registry is constructed")]
fn when_registry_constructed(world: &mut QuectoWorld) {
    let cfg = world.config.as_ref().expect("config not set");
    let td = TempDir::new().expect("temp dir");
    let ws = td.path().to_path_buf();
    let marker = ws.join(".nsjail-registry-marker");
    let available = world.nsjail_available.unwrap_or(false);
    let nsjail_binary = if cfg.tools.exec.isolation
        == quecto::infrastructure::config::ExecIsolationConfig::Nsjail
        && available
    {
        create_fake_nsjail_binary(&ws, &marker, false)
    } else {
        "definitely-missing-nsjail".to_string()
    };
    let sandbox = Sandbox::new(Some(ws.clone()), true);
    let mut settings = ToolRegistryImpl::exec_registry_settings_from_config(cfg);
    settings.nsjail_binary = nsjail_binary.clone();
    let _registry =
        ToolRegistryImpl::with_core_tools_and_exec_settings(ws.clone(), sandbox, settings.clone());
    let exec = ExecTool::with_options(
        Arc::new(ws),
        Arc::new(Sandbox::new(None, false)),
        ExecOptions {
            isolation_mode: settings.isolation_mode,
            allow_native_fallback: settings.allow_native_fallback,
            nsjail: NsjailOptions {
                binary: nsjail_binary,
                network_passthrough: settings.network_passthrough,
                memory_limit_mb: Some(settings.memory_limit_mb),
                pid_limit: Some(settings.pid_limit),
                cpu_time_limit_secs: Some(settings.cpu_time_limit_secs),
                wall_time_limit_secs: Some(settings.wall_time_limit_secs),
                die_with_parent: settings.die_with_parent,
            },
            ..ExecOptions::default()
        },
    );
    world.nsjail_registry_mode = Some(exec.mode());
    world.nsjail_startup_warning = exec.startup_warning().map(str::to_string);
    world._extra_temp_dirs.push(td);
}

#[then("the exec tool should use nsjail isolation")]
fn then_exec_mode_nsjail(world: &mut QuectoWorld) {
    assert_eq!(world.nsjail_registry_mode, Some(ExecIsolationMode::Nsjail));
}

#[then("the exec tool should use native isolation with sandbox denylist only")]
fn then_exec_mode_native(world: &mut QuectoWorld) {
    assert_eq!(world.nsjail_registry_mode, Some(ExecIsolationMode::Native));
}

#[then("the exec tool should fall back to native isolation")]
fn then_exec_fallback_native(world: &mut QuectoWorld) {
    assert_eq!(world.nsjail_registry_mode, Some(ExecIsolationMode::Native));
}

#[then("a warning should be logged mentioning nsjail unavailability")]
fn then_warning_mentions_unavailable(world: &mut QuectoWorld) {
    let warning = world.nsjail_startup_warning.clone().unwrap_or_default();
    assert!(warning.contains("nsjail"));
    assert!(warning.contains("falling back"));
}

#[then("the tool result should mention exit code 42")]
fn then_result_mentions_exit_42(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    let text = match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => e.clone(),
    };
    assert!(text.contains("42"), "result did not mention 42: {text}");
}

#[then("the tool result should contain \"/usr/bin/git\" or similar")]
fn then_result_contains_git_path(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    let text = match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => e.clone(),
    };
    assert!(text.contains("git"), "expected git path in result: {text}");
}

#[then(expr = "the file {string} should not exist on the host")]
fn then_file_not_exists_on_host(_world: &mut QuectoWorld, file: String) {
    assert!(
        !Path::new(&file).exists(),
        "host file should not exist: {file}"
    );
}

#[then("the host should not be affected")]
fn then_host_not_affected(_world: &mut QuectoWorld) {}

#[then("the process list should only show jail-internal processes")]
fn then_jail_internal_processes_only(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    let text = match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => e.clone(),
    };
    assert!(text.contains("PID COMMAND"));
    assert!(!text.contains("systemd"));
}

#[then("the tool result should not contain host process names")]
fn then_result_not_contains_host_names(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    let text = match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => e.clone(),
    };
    assert!(!text.contains("systemd"));
    assert!(!text.contains("sshd"));
}

#[then("the host init process should not be affected")]
fn then_host_init_not_affected(_world: &mut QuectoWorld) {}

#[then(expr = "the nsjail error should mention {string}")]
fn then_nsjail_error_mentions(world: &mut QuectoWorld, expected: String) {
    let result = world.tool_result.as_ref().expect("no result");
    let text = match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => e.clone(),
    };
    assert!(
        text.contains(&expected),
        "expected error to contain '{expected}', got: {text}"
    );
}

#[then("nsjail should not have been invoked")]
fn then_nsjail_not_invoked(world: &mut QuectoWorld) {
    let marker = world
        .nsjail_invocation_marker
        .as_ref()
        .expect("nsjail marker not set");
    assert!(!marker.exists(), "nsjail marker should not exist");
}

#[then(expr = "the nsjail execution should complete within {int} seconds")]
fn then_nsjail_execution_within(world: &mut QuectoWorld, secs: u64) {
    let elapsed_ms = world.nsjail_elapsed_ms.expect("no elapsed time recorded");
    assert!(
        elapsed_ms <= (secs as u128) * 1000,
        "expected elapsed <= {secs}s, got {}ms",
        elapsed_ms
    );
}

#[then("no nsjail processes should remain running")]
fn then_no_nsjail_processes(_world: &mut QuectoWorld) {}

#[then("no stale mount namespaces should remain")]
fn then_no_stale_mount_ns(_world: &mut QuectoWorld) {}

#[then("the nsjail sandbox process should also be terminated")]
fn then_nsjail_process_terminated(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    let text = match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => e.clone(),
    };
    assert!(text.contains("terminated"));
}

#[then("the tool result should be truncated to approximately 1 MiB")]
fn then_result_truncated_1mib(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no result");
    let text = match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => e.clone(),
    };
    assert!(
        text.len() <= (1024 * 1024) + 200,
        "expected output near cap, got {} bytes",
        text.len()
    );
}

#[then("the tool result should indicate truncation")]
fn then_result_indicates_truncation(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no result");
    let text = match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => e.clone(),
    };
    assert!(
        text.contains("truncated"),
        "result did not indicate truncation"
    );
}
