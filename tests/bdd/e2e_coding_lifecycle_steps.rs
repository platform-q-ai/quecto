// Step definitions for e2e_coding_lifecycle.feature
//
// Tests the full coding job pipeline: agent → coding_job tool → coordinator
// → lifecycle driver tick → repo clone → worker launch → result.
//
// Assertions use filesystem artifacts (mirror dirs, job dirs, session files)
// rather than parsing stdout, since the agent's stdout only contains the
// final LLM text response, not tool results.

use crate::QuectoWorld;
use cucumber::{given, then, when};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Given: test git repo in workspace
// ---------------------------------------------------------------------------

/// Create a real git repo that the coding_job tool can reference as "test-repo".
/// The repo lives inside the e2e workspace directory so WorkspaceRepoValidator
/// accepts it.
#[given("a test git repo in the e2e workspace")]
fn given_test_git_repo(world: &mut QuectoWorld) {
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("base dir should be set");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    let repo_dir = workspace.join("test-repo");
    std::fs::create_dir_all(&repo_dir).expect("create repo dir");

    // git init + initial commit so it's a valid repo with a "main" branch
    let run = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git command should execute");
        assert!(status.success(), "git {:?} failed", args);
    };

    run(&["init", "--quiet", "--initial-branch=main"]);
    run(&[
        "-c",
        "user.email=test@test.com",
        "-c",
        "user.name=Test",
        "commit",
        "--allow-empty",
        "-m",
        "init",
    ]);

    // Write a Cargo.toml so workers have something to edit
    std::fs::write(
        repo_dir.join("Cargo.toml"),
        "[package]\nname = \"test-repo\"\nversion = \"0.1.0\"\n",
    )
    .expect("write Cargo.toml");
    run(&["add", "."]);
    run(&[
        "-c",
        "user.email=test@test.com",
        "-c",
        "user.name=Test",
        "commit",
        "-m",
        "add Cargo.toml",
    ]);
}

// ---------------------------------------------------------------------------
// Given: job directory with cloned repo (for worker subprocess tests)
// ---------------------------------------------------------------------------

#[given("a job directory with a cloned test repo")]
fn given_job_dir_with_repo(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("create temp dir");
    let job_dir = td.path().join("job_001").join("repo");
    std::fs::create_dir_all(&job_dir).expect("create job repo dir");

    let run = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&job_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git command should execute");
        assert!(status.success(), "git {:?} failed", args);
    };

    run(&["init", "--quiet", "--initial-branch=main"]);
    run(&[
        "-c",
        "user.email=test@test.com",
        "-c",
        "user.name=Test",
        "commit",
        "--allow-empty",
        "-m",
        "init",
    ]);

    world.e2e_coding_job_dir = Some(td.path().join("job_001"));
    world._e2e_coding_job_temp = Some(td);
}

// ---------------------------------------------------------------------------
// Given: mock LLM for worker subprocess tests
// ---------------------------------------------------------------------------

/// Mount a wiremock mock that returns a simple text response, and rewrite
/// the config to point at it. This prepares the mock LLM for the worker
/// subprocess test so `cmd_worker_from_config` hits the mock server.
#[given(expr = "a mock LLM that returns text {string}")]
fn given_mock_llm_text_for_worker(world: &mut QuectoWorld, text: String) {
    use crate::e2e_steps::rewrite_config_to_uri;

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let response_body = serde_json::json!({
            "id": "chatcmpl-worker",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": text
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
    world.e2e_coding_worker_mock_text = Some(text);
}

// ---------------------------------------------------------------------------
// When: run quecto worker subprocess
// ---------------------------------------------------------------------------

#[when(expr = "I run quecto worker with run-id {string} job-id {string} and goal {string}")]
fn when_run_worker(world: &mut QuectoWorld, run_id: String, job_id: String, goal: String) {
    let job_dir = world
        .e2e_coding_job_dir
        .as_ref()
        .expect("job directory should be set")
        .to_string_lossy()
        .to_string();

    let args = vec![
        "quecto".to_string(),
        "worker".to_string(),
        "--run-id".to_string(),
        run_id,
        "--job-id".to_string(),
        job_id,
        "--job-dir".to_string(),
        job_dir,
        "--goal".to_string(),
        goal,
    ];

    let output = quecto::interface::cli::run_with_output(args, &world.cli_context);
    world.e2e_coding_worker_exit_code = Some(output.exit_code);
    world.e2e_coding_worker_stdout = Some(output.stdout);
    world.e2e_coding_worker_stderr = Some(output.stderr);
}

// ---------------------------------------------------------------------------
// Then: filesystem-based assertions
// ---------------------------------------------------------------------------

/// Check that a bare mirror directory exists for the given repo name.
/// The mirror lives under `<base_dir>/coding/mirrors/<safe_name>.git/`.
#[then(expr = "a mirror should exist for repo {string} in the coding cache")]
fn then_mirror_exists(world: &mut QuectoWorld, repo: String) {
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("base dir should be set");
    let mirror_dir = base
        .join("coding")
        .join("mirrors")
        .join(format!("{repo}.git"));
    assert!(
        mirror_dir.exists(),
        "Expected mirror directory at {:?} but it does not exist.\n\
         This means the lifecycle driver did not create a mirror during \
         job preparation.\nstderr:\n{}",
        mirror_dir,
        world.stderr
    );
}

/// Check that at least one job directory exists in the coding cache.
#[then("a job directory should exist in the coding cache")]
fn then_job_dir_exists(world: &mut QuectoWorld) {
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("base dir should be set");
    let jobs_dir = base.join("coding").join("jobs");
    assert!(
        jobs_dir.exists(),
        "Coding jobs directory does not exist at {:?}.\n\
         The lifecycle driver never created a job directory.\n\
         stderr:\n{}",
        jobs_dir,
        world.stderr
    );
    let entries: Vec<_> = std::fs::read_dir(&jobs_dir)
        .expect("read coding jobs dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    assert!(
        !entries.is_empty(),
        "No job directories found under {:?}.\n\
         The lifecycle driver created the jobs/ parent but no job was prepared.\n\
         stderr:\n{}",
        jobs_dir,
        world.stderr
    );
}

/// Assert that stderr does not contain tool error indicators.
#[then("the agent should not have reported tool errors in stderr")]
fn then_no_tool_errors_in_stderr(world: &mut QuectoWorld) {
    let stderr = &world.stderr;
    let error_patterns = [
        "is_error",
        "tool error",
        "Tool execution failed",
        "unknown tool",
    ];
    for pattern in &error_patterns {
        assert!(
            !stderr.contains(pattern),
            "Found tool error indicator '{}' in stderr.\n\
             The coding_job tool may not be registered or returned an error.\n\
             stderr:\n{}",
            pattern,
            stderr
        );
    }
}

/// Assert that the saved session file contains a tool result with the
/// given substring (e.g. "job_id"). The session file lives at
/// `<base_dir>/sessions/<key>.json`.
///
/// The session name from the feature file is the `-s` flag value.
/// `cmd_agent` prepends `cli:` to form the full key, and the store
/// replaces `:` with `_` for the filename.
#[then(expr = "the saved session {string} should contain a tool result with {string}")]
fn then_session_contains_tool_result(
    world: &mut QuectoWorld,
    session_name: String,
    expected: String,
) {
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("base dir should be set");
    // cmd_agent builds key as "cli:<name>", stored as "cli_<name>.json"
    let full_key = format!("cli:{}", session_name);
    let filename = format!("{}.json", full_key.replace(':', "_"));
    let session_path = base.join("sessions").join(&filename);
    assert!(
        session_path.exists(),
        "Session file {:?} does not exist. The agent did not persist the session.\n\
         stderr:\n{}",
        session_path,
        world.stderr
    );

    let content = std::fs::read_to_string(&session_path).expect("read session file");
    let session: serde_json::Value = serde_json::from_str(&content).expect("parse session JSON");

    // Look for tool-role messages containing the expected substring.
    let messages = session["messages"]
        .as_array()
        .expect("session should have messages array");
    let has_tool_result = messages.iter().any(|msg| {
        let role = msg["role"].as_str().unwrap_or("");
        let content_str = msg["content"].as_str().unwrap_or("");
        role == "tool" && content_str.contains(&expected)
    });
    assert!(
        has_tool_result,
        "No tool-role message containing '{}' found in session '{}'.\n\
         The coding_job tool result was not persisted in the session.\n\
         messages: {:?}",
        expected, session_name, messages
    );
}

// ---------------------------------------------------------------------------
// Then: worker event assertions
// ---------------------------------------------------------------------------

/// Check that the worker's stdout contains a JSON Lines event with the
/// given `type` field (e.g. "log.message").
#[then(expr = "the worker stdout should contain a JSON Lines event with type {string}")]
fn then_worker_stdout_contains_event(world: &mut QuectoWorld, event_type: String) {
    let stdout = world
        .e2e_coding_worker_stdout
        .as_ref()
        .expect("worker stdout should be captured");
    assert!(
        stdout.contains(&format!("\"type\":\"{}\"", event_type))
            || stdout.contains(&format!("\"type\": \"{}\"", event_type))
            || stdout.contains(&format!("\"event_type\":\"{}\"", event_type))
            || stdout.contains(&format!("\"event_type\": \"{}\"", event_type)),
        "Expected JSON Lines event with type '{}' in worker stdout.\n\
         The quecto worker subcommand may not be running the full \
         worker loop (cmd_worker_with_deps).\n\
         stdout:\n{}\nstderr:\n{}",
        event_type,
        stdout,
        world.e2e_coding_worker_stderr.as_deref().unwrap_or("")
    );
}

#[then(expr = "the worker exit code should be {int}")]
fn then_worker_exit_code(world: &mut QuectoWorld, expected: i32) {
    let actual = world
        .e2e_coding_worker_exit_code
        .expect("worker exit code should be captured");
    assert_eq!(
        actual,
        expected,
        "Expected worker exit code {}, got {}.\nstderr:\n{}",
        expected,
        actual,
        world.e2e_coding_worker_stderr.as_deref().unwrap_or("")
    );
}
