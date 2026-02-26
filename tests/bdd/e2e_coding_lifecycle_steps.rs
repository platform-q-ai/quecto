// Step definitions for e2e_coding_lifecycle.feature
//
// Tests the full coding job pipeline: agent → coding_job tool → coordinator
// → lifecycle driver tick → repo clone → worker launch → result.

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

#[given(expr = "a mock LLM that returns text {string}")]
fn given_mock_llm_text_for_worker(world: &mut QuectoWorld, text: String) {
    // Store text for worker subprocess — the worker test will mount this
    // via wiremock when it launches.
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
// Then: coding job tool response assertions
// ---------------------------------------------------------------------------

/// Extract the coding_job tool response JSON from stdout.
/// The agent prints tool results inline. We look for the JSON blob returned
/// by the coding_job status call, which contains a "state" field.
fn find_status_response_in_stdout(stdout: &str) -> Option<String> {
    // The tool result appears in stdout. Look for JSON containing "state":
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.contains("\"state\"") && trimmed.contains("\"job_id\"") {
            return Some(trimmed.to_string());
        }
    }
    // Also check for embedded JSON in prose (the LLM may echo it)
    if stdout.contains("\"state\"") {
        return Some(stdout.to_string());
    }
    None
}

#[then("the coding job tool should have returned a status response")]
fn then_tool_returned_status(world: &mut QuectoWorld) {
    let stdout = &world.stdout;
    assert!(
        find_status_response_in_stdout(stdout).is_some(),
        "Expected the coding_job tool to return a status response with a \"state\" field \
         and \"job_id\" field in stdout, but none found.\n\
         This means either:\n\
         - The coding_job tool is not registered in the CLI agent\n\
         - The LLM's tool call was not routed to the coding_job tool\n\
         - The status call returned an error\n\
         stdout:\n{}",
        stdout
    );
}

#[then(expr = "the coding job status in the tool response should not be {string}")]
fn then_status_not(world: &mut QuectoWorld, unexpected: String) {
    let stdout = &world.stdout;
    let response = find_status_response_in_stdout(stdout).unwrap_or_else(|| {
        panic!(
            "No status response found in stdout. The coding_job tool \
             did not return a response with a \"state\" field.\nstdout:\n{}",
            stdout
        )
    });
    let is_unexpected = response.contains(&format!("\"state\":\"{}\"", unexpected))
        || response.contains(&format!("\"state\": \"{}\"", unexpected));
    assert!(
        !is_unexpected,
        "Expected coding job status to NOT be '{}', but the tool response contains it.\n\
         This means the lifecycle driver did not tick the job forward.\n\
         The coordinator created the job but nothing advanced it past '{}'.\n\
         response: {}\nfull stdout:\n{}",
        unexpected, unexpected, response, stdout
    );
}

#[then(expr = "the coding job status in the tool response should be {string}")]
fn then_status_is(world: &mut QuectoWorld, expected: String) {
    let stdout = &world.stdout;
    let response = find_status_response_in_stdout(stdout)
        .unwrap_or_else(|| panic!("No status response found in stdout.\nstdout:\n{}", stdout));
    assert!(
        response.contains(&format!("\"state\":\"{}\"", expected))
            || response.contains(&format!("\"state\": \"{}\"", expected)),
        "Expected coding job status '{}' in tool response, but not found.\n\
         response: {}\nfull stdout:\n{}",
        expected,
        response,
        stdout
    );
}

#[then(expr = "the coding job status in the tool response should be one of {string}")]
fn then_status_one_of(world: &mut QuectoWorld, states: String) {
    let stdout = &world.stdout;
    let response = find_status_response_in_stdout(stdout)
        .unwrap_or_else(|| panic!("No status response found in stdout.\nstdout:\n{}", stdout));
    let valid: Vec<&str> = states.split(',').collect();
    let found = valid.iter().any(|s| {
        response.contains(&format!("\"state\":\"{}\"", s))
            || response.contains(&format!("\"state\": \"{}\"", s))
    });
    assert!(
        found,
        "Expected coding job status to be one of [{}], but not found.\n\
         response: {}\nfull stdout:\n{}",
        states, response, stdout
    );
}

// ---------------------------------------------------------------------------
// Then: file creation assertions for job repos
// ---------------------------------------------------------------------------

#[then(expr = "the coding job should have created a file {string} in the job repo")]
fn then_file_in_job_repo(world: &mut QuectoWorld, filename: String) {
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("base dir should be set");
    // The coding system stores job repos under <base>/coding/jobs/<job_id>/repo/
    let coding_dir = base.join("coding").join("jobs");
    assert!(
        coding_dir.exists(),
        "Coding jobs directory does not exist at {:?}.\n\
         The lifecycle driver never created a job repo.\n\
         This means the CLI agent is not ticking the lifecycle driver.",
        coding_dir
    );

    // Find the first job directory
    let entries: Vec<_> = std::fs::read_dir(&coding_dir)
        .expect("read coding jobs dir")
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !entries.is_empty(),
        "No job directories found under {:?}. Worker never ran.",
        coding_dir
    );

    let job_dir = &entries[0].path();
    let repo_dir = job_dir.join("repo");
    let file_path = repo_dir.join(&filename);
    assert!(
        file_path.exists(),
        "Expected file '{}' in job repo {:?}, but it does not exist.\n\
         The worker either didn't run or didn't create the file.",
        filename,
        repo_dir
    );
}

// ---------------------------------------------------------------------------
// Then: coding job list assertions
// ---------------------------------------------------------------------------

#[then(expr = "the coding job tool should have returned a list with {int} jobs")]
fn then_list_has_n_jobs(world: &mut QuectoWorld, expected: usize) {
    let stdout = &world.stdout;
    // The list response is JSON with a "jobs" array.
    // Count occurrences of "job_id" in stdout as a proxy.
    let count = stdout.matches("\"job_id\"").count();
    assert!(
        count >= expected,
        "Expected at least {} jobs in the list response, but found {} job_id occurrences.\n\
         This means either:\n\
         - The coding_job tool did not return a list response\n\
         - Not all jobs were created successfully\n\
         stdout:\n{}",
        expected,
        count,
        stdout
    );
}

// ---------------------------------------------------------------------------
// Then: worker event assertions
// ---------------------------------------------------------------------------

#[then(expr = "the worker stdout should contain a {string} event")]
fn then_worker_stdout_contains_event(world: &mut QuectoWorld, event_type: String) {
    let stdout = world
        .e2e_coding_worker_stdout
        .as_ref()
        .expect("worker stdout should be captured");
    assert!(
        stdout.contains(&format!("\"event_type\":\"{}\"", event_type))
            || stdout.contains(&format!("\"event_type\": \"{}\"", event_type)),
        "Expected '{}' event in worker stdout, but not found.\n\
         This means the quecto worker subcommand is not running the full \
         worker loop (cmd_worker_with_deps). It's likely still using the stub.\n\
         stdout:\n{}",
        event_type,
        stdout
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
