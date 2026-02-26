use cucumber::{given, then, when};
use quecto::application::coding_coordinator::{
    CodingCoordinator, CoordinatorPolicy, FailureInfo, SuccessInfo,
};
use quecto::domain::coding_command::RunRequest;
use quecto::domain::coding_job::ErrorCode;
use quecto::domain::coding_ports::{WorkerLaunchConfig, WorkerRuntime};
use quecto::infrastructure::coding::worker_runtime::MockWorkerRuntime;

use crate::{BddRepoValidator, BddSkillResolver, QuectoWorld};

// ── helpers ──────────────────────────────────────────────────────────────

fn ensure_cwl(world: &mut QuectoWorld) {
    if world.cwl_coordinator.is_none() {
        let validator = BddRepoValidator::default();
        let resolver = BddSkillResolver::default();
        let policy = CoordinatorPolicy::default();
        world.cwl_coordinator = Some(CodingCoordinator::new(validator, resolver, policy));
    }
    if world.cwl_worker_runtime.is_none() {
        world.cwl_worker_runtime = Some(MockWorkerRuntime::new());
    }
}

fn cwl_coord(
    world: &mut QuectoWorld,
) -> &mut CodingCoordinator<BddRepoValidator, BddSkillResolver> {
    world.cwl_coordinator.as_mut().expect("cwl coordinator")
}

fn cwl_runtime(world: &mut QuectoWorld) -> &mut MockWorkerRuntime {
    world.cwl_worker_runtime.as_mut().expect("cwl runtime")
}

fn default_launch_config(job_dir: &str, goal: &str) -> WorkerLaunchConfig {
    WorkerLaunchConfig {
        run_id: "run_test".to_string(),
        job_id: "job_test".to_string(),
        job_dir: job_dir.to_string(),
        goal: goal.to_string(),
        max_memory_mb: 512,
        max_cpu_seconds: 120,
        max_wall_seconds: 300,
        max_pids: 128,
        network_allowed_hosts: vec![],
        die_with_parent: true,
    }
}

fn last_cwl_job_id(world: &QuectoWorld) -> String {
    world.cwl_job_ids.last().expect("at least one job").clone()
}

fn parse_error_code(code_str: &str) -> ErrorCode {
    match code_str {
        "timeout" => ErrorCode::Timeout,
        "oom" => ErrorCode::Oom,
        "seccomp_violation" => ErrorCode::SeccompViolation,
        "tool_error" => ErrorCode::ToolError,
        "llm_refusal" => ErrorCode::LlmRefusal,
        "internal" => ErrorCode::Internal,
        "coordinator_crash" => ErrorCode::CoordinatorCrash,
        other => panic!("unknown error code: {other}"),
    }
}

// ── Given steps ─────────────────────────────────────────────────────────

#[given("a coordinator with a worker runtime for lifecycle tests")]
fn given_coordinator_with_runtime(world: &mut QuectoWorld) {
    ensure_cwl(world);
}

#[given(regex = r#"^a repo validator that accepts "([^"]+)" at ref "([^"]+)"$"#)]
fn given_repo_validator(world: &mut QuectoWorld, repo: String, base_ref: String) {
    let validator = BddRepoValidator {
        valid_repos: vec![repo.clone()],
        valid_refs: vec![(repo, base_ref)],
    };
    let resolver = BddSkillResolver::default();
    let policy = CoordinatorPolicy::default();
    world.cwl_coordinator = Some(CodingCoordinator::new(validator, resolver, policy));
    world.cwl_worker_runtime = Some(MockWorkerRuntime::new());
}

// ── When steps ──────────────────────────────────────────────────────────

#[when(regex = r#"^a coding job is submitted for "([^"]+)" at "([^"]+)" with goal "([^"]+)"$"#)]
fn when_job_submitted(world: &mut QuectoWorld, repo: String, base_ref: String, goal: String) {
    let req = RunRequest {
        goal,
        repo,
        base_ref,
        priority: Default::default(),
        profile: "default".to_string(),
        max_wall_seconds: None,
        labels: vec![],
        skills: vec![],
    };
    let resp = cwl_coord(world).run(req).expect("run should succeed");
    world.cwl_job_ids.push(resp.job_id);
}

#[when(
    regex = r#"^a second coding job is submitted for "([^"]+)" at "([^"]+)" with goal "([^"]+)"$"#
)]
fn when_second_job_submitted(
    world: &mut QuectoWorld,
    repo: String,
    base_ref: String,
    goal: String,
) {
    when_job_submitted(world, repo, base_ref, goal);
}

#[when("the coordinator begins preparation for the job")]
fn when_begin_preparation(world: &mut QuectoWorld) {
    let job_id = last_cwl_job_id(world);
    cwl_coord(world)
        .begin_preparation(&job_id)
        .expect("begin_preparation should succeed");
}

#[when("the coordinator begins preparation for both lifecycle jobs")]
fn when_begin_preparation_both(world: &mut QuectoWorld) {
    let ids: Vec<String> = world.cwl_job_ids.clone();
    for job_id in &ids {
        cwl_coord(world)
            .begin_preparation(job_id)
            .expect("begin_preparation should succeed");
    }
}

#[when(regex = r#"^the repo clone succeeds with duration (\d+)ms$"#)]
fn when_clone_succeeds(world: &mut QuectoWorld, duration_ms: u64) {
    let job_id = last_cwl_job_id(world);
    let goal = cwl_coord(world)
        .job(&job_id)
        .expect("job exists")
        .goal
        .clone();
    let job_dir = format!("/tmp/jobs/{}/repo", job_id);
    let config = default_launch_config(&job_dir, &goal);
    let pid = cwl_runtime(world)
        .launch(&config)
        .expect("launch should succeed");
    world.cwl_worker_pids.push(pid);
    cwl_coord(world)
        .mark_ready(&job_id, pid, Some(duration_ms))
        .expect("mark_ready should succeed");
}

#[when(regex = r#"^the repo clone fails with error "([^"]+)"$"#)]
fn when_clone_fails(world: &mut QuectoWorld, error: String) {
    world.cwl_clone_error = Some(error);
}

#[when("both lifecycle repo clones succeed")]
fn when_both_clones_succeed(world: &mut QuectoWorld) {
    let ids: Vec<String> = world.cwl_job_ids.clone();
    for job_id in &ids {
        let goal = cwl_coord(world).job(job_id).expect("job").goal.clone();
        let job_dir = format!("/tmp/jobs/{}/repo", job_id);
        let config = default_launch_config(&job_dir, &goal);
        let pid = cwl_runtime(world).launch(&config).expect("launch");
        world.cwl_worker_pids.push(pid);
    }
}

#[when("the coordinator marks the job ready with worker PID")]
fn when_mark_ready(world: &mut QuectoWorld) {
    // Already done in clone_succeeds — this step is for explicit scenarios
    // where clone + ready are separate. If PID is already set, skip.
    if world.cwl_worker_pids.is_empty() {
        let job_id = last_cwl_job_id(world);
        let goal = cwl_coord(world).job(&job_id).expect("job").goal.clone();
        let config = default_launch_config(&format!("/tmp/jobs/{}/repo", job_id), &goal);
        let pid = cwl_runtime(world).launch(&config).expect("launch");
        world.cwl_worker_pids.push(pid);
        cwl_coord(world)
            .mark_ready(&job_id, pid, None)
            .expect("mark_ready");
    }
}

#[when("the coordinator marks both lifecycle jobs ready")]
fn when_mark_both_ready(world: &mut QuectoWorld) {
    let ids: Vec<String> = world.cwl_job_ids.clone();
    let pids: Vec<u32> = world.cwl_worker_pids.clone();
    for (i, job_id) in ids.iter().enumerate() {
        cwl_coord(world)
            .mark_ready(job_id, pids[i], Some(100))
            .expect("mark_ready");
    }
}

#[when(regex = r#"^the worker emits a lifecycle status event "([^"]+)"$"#)]
fn when_worker_emits_status(world: &mut QuectoWorld, state: String) {
    let job_id = last_cwl_job_id(world);
    cwl_coord(world)
        .emit_worker_event(
            &job_id,
            "job.status",
            serde_json::json!({"state": state, "summary": state}),
        )
        .expect("emit_worker_event should succeed");
}

#[when(regex = r#"^the worker emits a lifecycle ([^ ]+) event for "([^"]+)"$"#)]
fn when_worker_emits_tool_event(world: &mut QuectoWorld, event_type: String, tool: String) {
    let job_id = last_cwl_job_id(world);
    cwl_coord(world)
        .emit_worker_event(
            &job_id,
            &event_type,
            serde_json::json!({"tool": tool, "call_id": "call-1"}),
        )
        .expect("emit_worker_event should succeed");
}

#[when(regex = r#"^the lifecycle worker exits with status (\d+)$"#)]
fn when_worker_exits(world: &mut QuectoWorld, status: i32) {
    let pid = *world.cwl_worker_pids.last().expect("worker pid");
    let rt = cwl_runtime(world);
    rt.simulate_exit(pid, status);
}

#[when(regex = r#"^the coordinator marks the lifecycle job succeeded with summary "([^"]+)"$"#)]
fn when_mark_succeeded(world: &mut QuectoWorld, summary: String) {
    let job_id = last_cwl_job_id(world);
    cwl_coord(world)
        .mark_succeeded(SuccessInfo {
            job_id: &job_id,
            summary: &summary,
            artifacts: vec![],
            duration_ms: None,
        })
        .expect("mark_succeeded should succeed");
}

#[when(
    regex = r#"^the coordinator marks the lifecycle job succeeded with summary "([^"]+)" and artifacts "([^"]+)"$"#
)]
fn when_mark_succeeded_with_artifacts(
    world: &mut QuectoWorld,
    summary: String,
    artifacts_csv: String,
) {
    let job_id = last_cwl_job_id(world);
    let artifacts: Vec<String> = artifacts_csv
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    cwl_coord(world)
        .mark_succeeded(SuccessInfo {
            job_id: &job_id,
            summary: &summary,
            artifacts,
            duration_ms: None,
        })
        .expect("mark_succeeded should succeed");
}

#[when(regex = r#"^the coordinator marks the lifecycle job failed with code "([^"]+)"$"#)]
fn when_mark_failed(world: &mut QuectoWorld, code: String) {
    let job_id = last_cwl_job_id(world);
    let error_code = parse_error_code(&code);
    cwl_coord(world)
        .mark_failed(FailureInfo {
            job_id: &job_id,
            error_code,
            error_detail: &code,
            is_retriable: None,
            duration_ms: None,
        })
        .expect("mark_failed should succeed");
}

#[when(
    regex = r#"^the coordinator marks the lifecycle job failed with code "([^"]+)" and detail "([^"]+)"$"#
)]
fn when_mark_failed_with_detail(world: &mut QuectoWorld, code: String, detail: String) {
    let job_id = last_cwl_job_id(world);
    let error_code = parse_error_code(&code);
    cwl_coord(world)
        .mark_failed(FailureInfo {
            job_id: &job_id,
            error_code,
            error_detail: &detail,
            is_retriable: None,
            duration_ms: None,
        })
        .expect("mark_failed should succeed");
}

#[when("the coordinator kills the lifecycle worker due to timeout")]
fn when_kill_worker(world: &mut QuectoWorld) {
    let pid = *world.cwl_worker_pids.last().expect("worker pid");
    cwl_runtime(world).kill(pid).expect("kill should succeed");
}

#[when("the coordinator cleans up the lifecycle worker")]
fn when_cleanup_worker(world: &mut QuectoWorld) {
    let pid = *world.cwl_worker_pids.last().expect("worker pid");
    cwl_runtime(world).cleanup(pid);
}

#[when(regex = r#"^the coordinator cancels the lifecycle job with reason "([^"]+)"$"#)]
fn when_cancel_job(world: &mut QuectoWorld, _reason: String) {
    let job_id = last_cwl_job_id(world);
    // Kill the worker first
    let pid_to_kill = world.cwl_worker_pids.last().copied();
    if let Some(pid) = pid_to_kill {
        let _ = cwl_runtime(world).kill(pid);
    }
    cwl_coord(world)
        .cancel(&job_id)
        .expect("cancel should succeed");
}

#[when(regex = r#"^the coordinator records worker progress "([^"]+)" with completion (\d+)$"#)]
fn when_record_progress(world: &mut QuectoWorld, summary: String, progress: u32) {
    let job_id = last_cwl_job_id(world);
    cwl_coord(world)
        .record_worker_progress(&job_id, progress, &summary)
        .expect("record_worker_progress should succeed");
}

// ── Then steps ──────────────────────────────────────────────────────────

#[then(regex = r#"^the lifecycle job should be in state "([^"]+)"$"#)]
fn then_job_state(world: &mut QuectoWorld, expected: String) {
    let job_id = last_cwl_job_id(world);
    let job = cwl_coord(world).job(&job_id).expect("job should exist");
    assert_eq!(
        job.state.to_string(),
        expected,
        "job state should be '{expected}', got '{}'",
        job.state
    );
}

#[then("the lifecycle job should have a worker PID set")]
fn then_job_has_worker_pid(world: &mut QuectoWorld) {
    let job_id = last_cwl_job_id(world);
    let job = cwl_coord(world).job(&job_id).expect("job should exist");
    assert!(job.worker_pid.is_some(), "job should have a worker PID set");
}

#[then(regex = r#"^a lifecycle event with type "([^"]+)" should exist$"#)]
fn then_event_exists(world: &mut QuectoWorld, event_type: String) {
    let events = cwl_coord(world).events();
    assert!(
        events.iter().any(|e| e.event_type == event_type),
        "event with type '{event_type}' should exist in events"
    );
}

#[then(regex = r#"^the lifecycle job summary should contain "([^"]+)"$"#)]
fn then_summary_contains(world: &mut QuectoWorld, expected: String) {
    let job_id = last_cwl_job_id(world);
    let job = cwl_coord(world).job(&job_id).expect("job should exist");
    let summary = job.summary.as_deref().unwrap_or("");
    assert!(
        summary.contains(&expected),
        "summary should contain '{expected}', got '{summary}'"
    );
}

#[then(regex = r#"^the lifecycle events should include "([^"]+)" and "([^"]+)"$"#)]
fn then_events_include(world: &mut QuectoWorld, type1: String, type2: String) {
    let events = cwl_coord(world).events();
    assert!(
        events.iter().any(|e| e.event_type == type1),
        "events should include '{type1}'"
    );
    assert!(
        events.iter().any(|e| e.event_type == type2),
        "events should include '{type2}'"
    );
}

#[then("the lifecycle worker should not be alive")]
fn then_worker_not_alive(world: &mut QuectoWorld) {
    let pid = *world.cwl_worker_pids.last().expect("worker pid");
    let rt = cwl_runtime(world);
    assert!(!rt.is_alive(pid), "worker should not be alive");
}

#[then(regex = r#"^(\d+) lifecycle workers should be running$"#)]
fn then_n_workers_running(world: &mut QuectoWorld, expected: usize) {
    let rt = cwl_runtime(world);
    assert_eq!(
        rt.running_count(),
        expected,
        "expected {expected} running workers"
    );
}

#[then(regex = r#"^the lifecycle event count should be at least (\d+)$"#)]
fn then_event_count_at_least(world: &mut QuectoWorld, min: usize) {
    let count = cwl_coord(world).events().len();
    assert!(
        count >= min,
        "event count should be at least {min}, got {count}"
    );
}

#[then(regex = r#"^the lifecycle job status should include artifacts "([^"]+)" and "([^"]+)"$"#)]
fn then_status_includes_artifacts(world: &mut QuectoWorld, artifact1: String, artifact2: String) {
    let job_id = last_cwl_job_id(world);
    let status = cwl_coord(world)
        .status_by_job_id(&job_id)
        .expect("status should succeed");
    let artifacts = &status.artifacts;
    assert!(
        artifacts.iter().any(|a| a == &artifact1),
        "artifacts should include '{artifact1}'"
    );
    assert!(
        artifacts.iter().any(|a| a == &artifact2),
        "artifacts should include '{artifact2}'"
    );
}

#[then(regex = r#"^the lifecycle job status should include error_code "([^"]+)"$"#)]
fn then_status_error_code(world: &mut QuectoWorld, expected: String) {
    let job_id = last_cwl_job_id(world);
    let status = cwl_coord(world)
        .status_by_job_id(&job_id)
        .expect("status should succeed");
    let code = status.error_code.map(|c| c.to_string()).unwrap_or_default();
    assert_eq!(
        code, expected,
        "error_code should be '{expected}', got '{code}'"
    );
}

#[then(regex = r#"^the lifecycle job status should include error_detail "([^"]+)"$"#)]
fn then_status_error_detail(world: &mut QuectoWorld, expected: String) {
    let job_id = last_cwl_job_id(world);
    let status = cwl_coord(world)
        .status_by_job_id(&job_id)
        .expect("status should succeed");
    let detail = status.error_detail.as_deref().unwrap_or("");
    assert!(
        detail.contains(&expected),
        "error_detail should contain '{expected}', got '{detail}'"
    );
}
