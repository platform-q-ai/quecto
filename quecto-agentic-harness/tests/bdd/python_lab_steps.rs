use crate::{DebugPythonLab, QuectoWorld};
use cucumber::{given, then, when};
use quecto::domain::tool::{Tool, ToolResult};
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::python_lab::{PythonLabConfig, PythonLabTool};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Background `python_lab` jobs are `tokio::spawn`ed, so they only survive for
/// as long as the runtime that started them. A per-process runtime keeps those
/// tasks alive across the separate steps of a scenario; a runtime created and
/// dropped inside each step would silently kill every background job.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build python lab test runtime")
    })
}

fn ensure_workspace(world: &mut QuectoWorld) -> PathBuf {
    if world.python_lab_workspace.is_none() {
        let tmp = TempDir::new().expect("failed to create python lab temp dir");
        let path = tmp.path().to_path_buf();
        world._python_lab_temp_dir = Some(tmp);
        world.python_lab_workspace = Some(path);
    }
    world.python_lab_workspace.clone().unwrap()
}

/// One tool instance per scenario, so the background job registry it owns is
/// shared between the run/status/output/cancel steps.
fn tool(world: &mut QuectoWorld) -> Arc<PythonLabTool> {
    let ws = ensure_workspace(world);
    if world.python_lab_tool.is_none() {
        let sandbox = Arc::new(Sandbox::new(Some(ws.clone())));
        let tool = PythonLabTool::new(Arc::new(ws), sandbox, PythonLabConfig::default());
        tool.set_session_key("bdd-python-lab".into());
        world.python_lab_tool = Some(DebugPythonLab(Arc::new(tool)));
    }
    world.python_lab_tool.as_ref().unwrap().0.clone()
}

fn run(world: &mut QuectoWorld, args: serde_json::Value) {
    let tool = tool(world);
    let result = runtime()
        .block_on(async { tool.execute(&args.to_string()).await })
        .unwrap_or_else(|e| ToolResult {
            content: e.to_string(),
            is_error: true,
            image_blocks: vec![],
            delivery_metadata: None,
        });
    world.python_lab_result = Some(result);
}

fn result(world: &QuectoWorld) -> &ToolResult {
    world
        .python_lab_result
        .as_ref()
        .expect("expected a python lab result")
}

fn result_json(world: &QuectoWorld) -> serde_json::Value {
    serde_json::from_str(&result(world).content).unwrap_or_else(|e| {
        panic!(
            "python lab result should be JSON ({e}): {}",
            result(world).content
        )
    })
}

fn job_id(world: &QuectoWorld) -> String {
    world
        .python_lab_job_id
        .clone()
        .expect("expected a python lab job id")
}

// ---------------------------------------------------------------------------
// Given
// ---------------------------------------------------------------------------

#[given("a python lab workspace")]
fn given_workspace(world: &mut QuectoWorld) {
    ensure_workspace(world);
}

#[given(regex = r#"^a python lab workspace file "([^"]+)" with content:$"#)]
fn given_workspace_file(world: &mut QuectoWorld, step: &cucumber::gherkin::Step, filename: String) {
    let ws = ensure_workspace(world);
    let content = step
        .docstring
        .as_deref()
        .expect("step should carry a docstring")
        .trim_start_matches('\n');
    std::fs::write(ws.join(filename), content).expect("failed to write python lab file");
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

#[when(regex = r#"^I run python lab inline code "(.+)"$"#)]
fn when_run_inline(world: &mut QuectoWorld, code: String) {
    run(world, serde_json::json!({"op": "run", "code": code}));
}

#[when(regex = r#"^I run python lab inline code "(.+)" with timeout (\d+) seconds$"#)]
fn when_run_inline_with_timeout(world: &mut QuectoWorld, code: String, seconds: u64) {
    run(
        world,
        serde_json::json!({"op": "run", "code": code, "timeout_seconds": seconds}),
    );
}

#[when(regex = r#"^I run python lab inline code "(.+)" with max output (\d+) bytes$"#)]
fn when_run_inline_with_max_output(world: &mut QuectoWorld, code: String, bytes: u64) {
    run(
        world,
        serde_json::json!({"op": "run", "code": code, "max_output_bytes": bytes}),
    );
}

#[when(regex = r#"^I run python lab inline code "(.+)" in the background$"#)]
fn when_run_inline_background(world: &mut QuectoWorld, code: String) {
    run(
        world,
        serde_json::json!({"op": "run", "code": code, "background": true}),
    );
    if let Some(id) = result_json(world).get("job_id").and_then(|v| v.as_str()) {
        world.python_lab_job_id = Some(id.to_string());
    }
}

#[when(regex = r#"^I run python lab file "([^"]+)"$"#)]
fn when_run_file(world: &mut QuectoWorld, path: String) {
    run(world, serde_json::json!({"op": "run", "path": path}));
}

#[when(regex = r#"^I run python lab file "([^"]+)" with args "(.*)" and stdin "(.*)"$"#)]
fn when_run_file_with_args(world: &mut QuectoWorld, path: String, args: String, stdin: String) {
    let args: Vec<&str> = if args.is_empty() {
        vec![]
    } else {
        args.split(',').collect()
    };
    run(
        world,
        serde_json::json!({"op": "run", "path": path, "args": args, "stdin": stdin}),
    );
}

#[when("I run python lab with both code and path")]
fn when_run_both(world: &mut QuectoWorld) {
    run(
        world,
        serde_json::json!({"op": "run", "code": "print(1)", "path": "some.py"}),
    );
}

#[when("I run python lab with neither code nor path")]
fn when_run_neither(world: &mut QuectoWorld) {
    run(world, serde_json::json!({"op": "run"}));
}

#[when(regex = r#"^I run python lab op "([^"]+)"$"#)]
fn when_run_op(world: &mut QuectoWorld, op: String) {
    run(world, serde_json::json!({"op": op}));
}

#[when(regex = r#"^I ask for python lab status of job "([^"]+)"$"#)]
fn when_status_of(world: &mut QuectoWorld, id: String) {
    run(world, serde_json::json!({"op": "status", "job_id": id}));
}

#[when("I cancel the background python lab job")]
fn when_cancel(world: &mut QuectoWorld) {
    let id = job_id(world);
    run(world, serde_json::json!({"op": "cancel", "job_id": id}));
}

// ---------------------------------------------------------------------------
// Then
// ---------------------------------------------------------------------------

#[then(regex = r#"^the python lab result should contain "(.+)"$"#)]
fn then_contains(world: &mut QuectoWorld, needle: String) {
    let content = &result(world).content;
    assert!(
        content.contains(&needle),
        "expected python lab result to contain {needle:?}, got: {content}"
    );
}

#[then(regex = r#"^the python lab status should be "([^"]+)"$"#)]
fn then_status(world: &mut QuectoWorld, expected: String) {
    let actual = result_json(world)
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    assert_eq!(actual, expected, "unexpected python lab status");
}

/// Distinguishes a sandbox refusal from the tool merely failing for some other
/// reason — a rejected path and a path that simply does not exist both surface
/// as errors, so asserting only `is_error` would pass with no sandbox at all.
#[then("the python lab result should be a sandbox rejection")]
fn then_sandbox_rejection(world: &mut QuectoWorld) {
    let result = result(world);
    assert!(result.is_error, "expected an error: {}", result.content);
    assert!(
        result.content.contains("security violation"),
        "expected a sandbox rejection, got: {}",
        result.content
    );
}

#[then("the python lab result should be an error")]
fn then_is_error(world: &mut QuectoWorld) {
    assert!(
        result(world).is_error,
        "expected python lab result to be an error: {}",
        result(world).content
    );
}

#[then("the python lab result should not be an error")]
fn then_not_error(world: &mut QuectoWorld) {
    assert!(
        !result(world).is_error,
        "expected python lab result not to be an error: {}",
        result(world).content
    );
}

#[then("the python lab exit code should not be zero")]
fn then_nonzero_exit(world: &mut QuectoWorld) {
    let code = result_json(world)
        .get("exit_code")
        .and_then(|v| v.as_i64())
        .expect("result should carry an exit_code");
    assert_ne!(code, 0, "expected a non-zero exit code");
}

#[then(regex = r#"^the python lab result should list "([^"]+)" as modified$"#)]
fn then_lists_modified(world: &mut QuectoWorld, name: String) {
    let json = result_json(world);
    let changed = json
        .get("files_created_or_modified")
        .and_then(|v| v.as_array())
        .expect("result should carry files_created_or_modified");
    assert!(
        changed.iter().any(|v| v.as_str() == Some(name.as_str())),
        "expected {name:?} among changed files, got: {changed:?}"
    );
}

#[then(regex = r#"^the python lab result should report cancel reason "([^"]+)"$"#)]
fn then_cancel_reason(world: &mut QuectoWorld, expected: String) {
    let json = result_json(world);
    let actual = json
        .get("timeout_or_cancel_reason")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(actual, expected, "unexpected timeout_or_cancel_reason");
}

#[then("the python lab result should report truncated output")]
fn then_truncated(world: &mut QuectoWorld) {
    let json = result_json(world);
    assert_eq!(
        json.get("output_truncated").and_then(|v| v.as_bool()),
        Some(true),
        "expected output_truncated to be true: {json}"
    );
}

#[then("the python lab artifact should contain the full output")]
fn then_artifact_has_full_output(world: &mut QuectoWorld) {
    let json = result_json(world);
    let paths = json
        .get("artifact_paths")
        .and_then(|v| v.as_array())
        .expect("result should carry artifact_paths");
    let rel = paths
        .first()
        .and_then(|v| v.as_str())
        .expect("expected at least one artifact path");
    let ws = ensure_workspace(world);
    let full = std::fs::read_to_string(ws.join(rel)).expect("failed to read python lab artifact");
    let preview = json
        .get("stdout")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    // The scenario prints exactly 5000 'x' plus a newline. Asserting the whole
    // thing survived, rather than merely "more than the preview", is what makes
    // this prove the output is recoverable.
    assert_eq!(
        full,
        format!("{}\n", "x".repeat(5000)),
        "artifact should hold the complete output, got {} bytes",
        full.len()
    );
    assert!(
        full.len() > preview.len(),
        "preview should be shorter than the artifact ({} vs {})",
        preview.len(),
        full.len()
    );
}

#[then(regex = r#"^the python lab workspace should not contain "([^"]+)"$"#)]
fn then_workspace_lacks(world: &mut QuectoWorld, name: String) {
    let ws = ensure_workspace(world);
    assert!(
        !ws.join(&name).exists(),
        "workspace should not contain {name:?}"
    );
}

#[then("the python lab result should include audit metadata")]
fn then_audit_metadata(world: &mut QuectoWorld) {
    let json = result_json(world);
    for field in [
        "execution_id",
        "session_id",
        "invocation_type",
        "interpreter",
        "interpreter_version",
        "start_time_ms",
        "completion_time_ms",
        "duration_ms",
        "timeout_seconds",
        "resource_limits",
        "resource_usage",
        "files_created_or_modified",
    ] {
        // Rejects explicit nulls: `.is_some()` alone would pass for a field
        // that is present but carries no value.
        assert!(
            json.get(field).is_some_and(|v| !v.is_null()),
            "audit metadata should include a non-null {field}: {json}"
        );
    }
    let usage = &json["resource_usage"];
    assert!(
        usage
            .get("stdout_bytes_retained")
            .is_some_and(|v| v.is_u64()),
        "resource_usage should report stdout_bytes_retained: {usage}"
    );
    assert!(
        usage
            .get("stderr_bytes_retained")
            .is_some_and(|v| v.is_u64()),
        "resource_usage should report stderr_bytes_retained: {usage}"
    );
}

#[then("the python lab result should report a job id")]
fn then_has_job_id(world: &mut QuectoWorld) {
    let json = result_json(world);
    let id = json
        .get("job_id")
        .and_then(|v| v.as_str())
        .expect("expected a job_id in the background run result");
    assert!(!id.is_empty(), "job_id should not be empty");
    world.python_lab_job_id = Some(id.to_string());
}

#[then(regex = r#"^the background python lab job should reach status "([^"]+)"$"#)]
fn then_job_reaches_status(world: &mut QuectoWorld, expected: String) {
    let id = job_id(world);
    // Kept short deliberately: cucumber drives every step on one executor
    // thread, so a stalled poll here blocks the whole shard against its
    // 5-minute CI budget. The jobs these scenarios run finish in milliseconds.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        run(world, serde_json::json!({"op": "status", "job_id": id}));
        let actual = result_json(world)
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if actual == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "background job never reached {expected:?} (last status {actual:?})"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn pid_is_alive(pid: i32) -> bool {
    // SAFETY: kill(pid, 0) only checks existence/permission, sending no signal.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Reads the pid the background program recorded for itself, waiting for the
/// file to appear.
fn recorded_pid(world: &mut QuectoWorld) -> i32 {
    let ws = ensure_workspace(world);
    let path = ws.join("pid.txt");
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(pid) = text.trim().parse::<i32>() {
                return pid;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("background program never recorded its pid at {path:?}");
}

#[then("the background python lab process should be running")]
fn then_process_running(world: &mut QuectoWorld) {
    let pid = recorded_pid(world);
    world.python_lab_pid = Some(pid);
    assert!(pid_is_alive(pid), "expected pid {pid} to be running");
}

/// Asserts the observable effect of cancellation rather than the wording of the
/// reply: a cancel that returned "cancelling" without killing anything would
/// still satisfy a response-only assertion.
#[then("the cancelled python lab process should no longer be running")]
fn then_process_dead(world: &mut QuectoWorld) {
    let pid = world
        .python_lab_pid
        .expect("expected a recorded pid from an earlier step");
    for _ in 0..100 {
        if !pid_is_alive(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("pid {pid} was still alive after cancellation");
}

#[then(regex = r#"^the background python lab output should contain "(.+)"$"#)]
fn then_job_output_contains(world: &mut QuectoWorld, needle: String) {
    let id = job_id(world);
    run(world, serde_json::json!({"op": "output", "job_id": id}));
    let content = &result(world).content;
    assert!(
        content.contains(&needle),
        "expected background output to contain {needle:?}, got: {content}"
    );
}
