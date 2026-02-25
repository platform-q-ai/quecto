use cucumber::{given, then, when};
use quecto::infrastructure::coding::worker_event_emitter::{EmitterConfig, WorkerEventEmitter};
use quecto::infrastructure::coding::worker_tool_wrappers::build_worker_tool_registry;
use quecto::interface::cli::worker::{WorkerArgs, parse_worker_args, validate_job_dir};
use quecto::interface::cli::{self, CliContext};
use tempfile::TempDir;

use crate::QuectoWorld;

// ── When steps: argument parsing ────────────────────────────────────────

#[when("I run quecto worker with flags:")]
fn when_parse_worker_flags(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    let table = step.table.as_ref().expect("expected a table");
    let mut args: Vec<String> = Vec::new();
    for row in &table.rows[1..] {
        args.push(row[0].trim().to_string());
        args.push(row[1].trim().to_string());
    }
    let result = parse_worker_args(&args);
    world.cwe_parsed_args = Some(result.map_err(|e| e.to_string()));
}

// ── Then steps: argument parsing ────────────────────────────────────────

#[then("the worker args should parse successfully")]
fn then_args_parse_ok(world: &mut QuectoWorld) {
    let result = world.cwe_parsed_args.as_ref().expect("no parse result");
    assert!(
        result.is_ok(),
        "expected Ok but got Err: {:?}",
        result.as_ref().err()
    );
}

#[then(expr = "the worker args should fail with {string}")]
fn then_args_parse_fail(world: &mut QuectoWorld, expected: String) {
    let result = world.cwe_parsed_args.as_ref().expect("no parse result");
    let err = result
        .as_ref()
        .err()
        .unwrap_or_else(|| panic!("expected Err but got Ok"));
    assert!(
        err.contains(&expected),
        "expected error to contain '{expected}' but got: {err}"
    );
}

#[then(expr = "the parsed run_id should be {string}")]
fn then_parsed_run_id(world: &mut QuectoWorld, expected: String) {
    let args = parsed_args(world);
    assert_eq!(args.run_id, expected);
}

#[then(expr = "the parsed job_id should be {string}")]
fn then_parsed_job_id(world: &mut QuectoWorld, expected: String) {
    let args = parsed_args(world);
    assert_eq!(args.job_id, expected);
}

#[then(expr = "the parsed goal should be {string}")]
fn then_parsed_goal(world: &mut QuectoWorld, expected: String) {
    let args = parsed_args(world);
    assert_eq!(args.goal, expected);
}

#[then(expr = "the parsed model should be {string}")]
fn then_parsed_model(world: &mut QuectoWorld, expected: String) {
    let args = parsed_args(world);
    assert_eq!(
        args.model.as_deref(),
        Some(expected.as_str()),
        "model mismatch"
    );
}

#[then("the parsed model should be empty")]
fn then_parsed_model_empty(world: &mut QuectoWorld) {
    let args = parsed_args(world);
    assert!(args.model.is_none(), "expected model to be None");
}

#[then(expr = "the parsed max_iterations should be {int}")]
fn then_parsed_max_iterations(world: &mut QuectoWorld, expected: u32) {
    let args = parsed_args(world);
    assert_eq!(args.max_iterations, Some(expected));
}

// ── Job directory validation ────────────────────────────────────────────

#[given("a temporary job directory with files:")]
fn given_temp_job_dir(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    let tmp = TempDir::new().unwrap();
    let job_dir = tmp.path().to_path_buf();
    let table = step.table.as_ref().expect("expected a table");
    for row in &table.rows[1..] {
        let path = row[0].trim();
        let content = row[1].trim().replace("\\n", "\n");
        let full = job_dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, &content).unwrap();
    }
    world.cwe_job_dir = Some(job_dir);
    world._cwe_temp_dir = Some(tmp);
}

#[when("I validate the worker job directory")]
fn when_validate_job_dir(world: &mut QuectoWorld) {
    let dir = world
        .cwe_job_dir
        .as_ref()
        .expect("job dir not set")
        .to_str()
        .unwrap()
        .to_string();
    world.cwe_validation_result = Some(validate_job_dir(&dir));
}

#[when(expr = "I validate a non-existent worker job directory {string}")]
fn when_validate_nonexistent(world: &mut QuectoWorld, path: String) {
    world.cwe_validation_result = Some(validate_job_dir(&path));
}

#[then("the worker job directory validation should succeed")]
fn then_validation_ok(world: &mut QuectoWorld) {
    let result = world.cwe_validation_result.as_ref().expect("no result");
    assert!(result.is_ok(), "expected Ok but got: {:?}", result);
}

#[then(expr = "the worker job directory validation should fail with {string}")]
fn then_validation_fail(world: &mut QuectoWorld, expected: String) {
    let result = world.cwe_validation_result.as_ref().expect("no result");
    let err = result
        .as_ref()
        .err()
        .unwrap_or_else(|| panic!("expected Err but got Ok"));
    assert!(
        err.contains(&expected),
        "expected error to contain '{expected}' but got: {err}"
    );
}

// ── Tool registry wiring ────────────────────────────────────────────────

#[when("I build the worker tool registry for the job directory")]
fn when_build_registry(world: &mut QuectoWorld) {
    let dir = world.cwe_job_dir.as_ref().expect("job dir not set").clone();
    let registry = build_worker_tool_registry(dir);
    world.cwe_registry = Some(registry);
}

#[then(expr = "the built worker registry should contain {string}")]
fn then_built_registry_contains(world: &mut QuectoWorld, name: String) {
    let reg = world.cwe_registry.as_ref().expect("registry not built");
    assert!(
        reg.get(&name).is_some(),
        "expected registry to contain '{name}'"
    );
}

// ── Event emitter wiring ────────────────────────────────────────────────

#[when(expr = "I create a worker event emitter for run {string} and job {string}")]
fn when_create_emitter(world: &mut QuectoWorld, run_id: String, job_id: String) {
    let emitter = WorkerEventEmitter::new(
        EmitterConfig {
            run_id,
            job_id,
            version: "1.0".to_string(),
        },
        Vec::new(),
    );
    world.cwe_emitter = Some(emitter);
}

#[then(expr = "the worker emitter should emit events with run_id {string}")]
fn then_emitter_run_id(world: &mut QuectoWorld, expected: String) {
    let emitter = world.cwe_emitter.as_mut().expect("emitter not created");
    emitter
        .emit(
            "log.message",
            serde_json::json!({"level": "info", "message": "test"}),
        )
        .unwrap();
    let output = String::from_utf8(emitter.writer().clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(output.lines().last().unwrap()).unwrap();
    assert_eq!(json["run_id"].as_str().unwrap(), expected);
}

#[then(expr = "the worker emitter should emit events with job_id {string}")]
fn then_emitter_job_id(world: &mut QuectoWorld, expected: String) {
    let emitter = world.cwe_emitter.as_ref().expect("emitter not created");
    let output = String::from_utf8(emitter.writer().clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(output.lines().last().unwrap()).unwrap();
    assert_eq!(json["job_id"].as_str().unwrap(), expected);
}

// ── CLI dispatch ────────────────────────────────────────────────────────

#[when(expr = "I run quecto with args {string}")]
fn when_run_quecto_with_args(world: &mut QuectoWorld, args_str: String) {
    let mut args: Vec<String> = vec!["quecto".to_string()];
    args.extend(args_str.split_whitespace().map(String::from));
    let output = cli::run_with_output(args, &CliContext::default());
    world.cwe_cli_stdout = Some(output.stdout);
    world.cwe_cli_stderr = Some(output.stderr);
    world.cwe_cli_exit_code = Some(output.exit_code);
}

#[then("the worker cli exit code should not indicate unknown command")]
fn then_not_unknown_command(world: &mut QuectoWorld) {
    let stderr = world.cwe_cli_stderr.as_ref().expect("no stderr");
    assert!(
        !stderr.contains("Unknown command"),
        "got 'Unknown command' in stderr: {stderr}"
    );
}

#[then(expr = "the worker cli output should not contain {string}")]
fn then_worker_cli_not_contain(world: &mut QuectoWorld, unexpected: String) {
    let stdout = world.cwe_cli_stdout.as_ref().expect("no stdout");
    let stderr = world.cwe_cli_stderr.as_ref().expect("no stderr");
    let combined = format!("{stdout}{stderr}");
    assert!(
        !combined.contains(&unexpected),
        "output should not contain '{unexpected}' but got: {combined}"
    );
}

#[then(expr = "the worker cli output should contain {string}")]
fn then_worker_cli_contain(world: &mut QuectoWorld, expected: String) {
    let stdout = world.cwe_cli_stdout.as_ref().expect("no stdout");
    assert!(
        stdout.contains(&expected),
        "expected stdout to contain '{expected}' but got: {stdout}"
    );
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn parsed_args(world: &QuectoWorld) -> &WorkerArgs {
    world
        .cwe_parsed_args
        .as_ref()
        .expect("no parse result")
        .as_ref()
        .expect("parse failed")
}
