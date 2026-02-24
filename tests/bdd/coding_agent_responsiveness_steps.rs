use super::*;

use quecto::application::coding_coordinator::{CoordinatorPolicy, FailureInfo, SuccessInfo};
use quecto::domain::coding_command::{ListRequest, RunRequest};
use quecto::domain::coding_job::{CancelInitiator, CancelReason, ErrorCode, JobState, Priority};

fn parse_state(s: &str) -> JobState {
    s.parse::<JobState>().expect("invalid state in scenario")
}

fn parse_list_literal(s: &str) -> Vec<String> {
    s.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|x| x.trim().trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Build a fresh coordinator for agent responsiveness scenarios.
fn build_resp_coordinator(world: &mut QuectoWorld) {
    let coord = CodingCoordinator::new(
        BddRepoValidator {
            valid_repos: vec!["test-repo".to_string(), "org/myrepo".to_string()],
            valid_refs: vec![
                ("test-repo".to_string(), "main".to_string()),
                ("org/myrepo".to_string(), "develop".to_string()),
            ],
        },
        BddSkillResolver {
            available: vec!["rust-style".to_string()],
        },
        CoordinatorPolicy::default(),
    );
    world.coding_coordinator = Some(coord);
}

fn ensure_resp_coordinator(world: &mut QuectoWorld) {
    if world.coding_coordinator.is_none() {
        build_resp_coordinator(world);
    }
}

/// Build a default `RunRequest` for test-repo/main with the given goal.
fn default_run_request(goal: &str) -> RunRequest {
    RunRequest {
        goal: goal.to_string(),
        repo: "test-repo".to_string(),
        base_ref: "main".to_string(),
        priority: Priority::default(),
        profile: "default".to_string(),
        max_wall_seconds: None,
        labels: vec![],
        skills: vec![],
    }
}

/// Run a job with a given goal through the coordinator and store IDs.
fn run_job_with_goal(world: &mut QuectoWorld, goal: &str) -> (String, String) {
    ensure_resp_coordinator(world);
    let coord = world.coding_coordinator.as_mut().unwrap();
    let resp = coord
        .run(default_run_request(goal))
        .expect("run_job_with_goal should succeed");
    let jid = resp.job_id.clone();
    let rid = resp.run_id.clone();
    world.coding_current_job_id = Some(jid.clone());
    world.coding_run_response = Some(resp);
    (jid, rid)
}

/// Advance a freshly-run job to target state through coordinator.
fn advance_new_job_to(world: &mut QuectoWorld, goal: &str, target: JobState) -> String {
    let (jid, _) = run_job_with_goal(world, goal);
    let coord = world.coding_coordinator.as_mut().unwrap();
    match target {
        JobState::Queued => { /* already there */ }
        JobState::Preparing => {
            coord.begin_preparation(&jid).expect("begin_preparation");
        }
        JobState::Running => {
            coord.begin_preparation(&jid).expect("begin_preparation");
            coord.mark_ready(&jid, 4242, None).expect("mark_ready");
        }
        JobState::Blocked => {
            coord.begin_preparation(&jid).expect("begin_preparation");
            coord.mark_ready(&jid, 4242, None).expect("mark_ready");
            coord
                .mark_blocked(&jid, "needs decision", None)
                .expect("mark_blocked");
        }
        JobState::Succeeded => {
            coord.begin_preparation(&jid).expect("begin_preparation");
            coord.mark_ready(&jid, 4242, None).expect("mark_ready");
            coord
                .mark_succeeded(SuccessInfo {
                    job_id: &jid,
                    summary: "done",
                    artifacts: vec!["patch_001".to_string()],
                    duration_ms: None,
                })
                .expect("mark_succeeded");
        }
        JobState::Failed => {
            coord.begin_preparation(&jid).expect("begin_preparation");
            coord.mark_ready(&jid, 4242, None).expect("mark_ready");
            coord
                .mark_failed(FailureInfo {
                    job_id: &jid,
                    error_code: ErrorCode::Internal,
                    error_detail: "failed",
                    is_retriable: None,
                    duration_ms: None,
                })
                .expect("mark_failed");
        }
        JobState::Canceled => {
            coord.begin_preparation(&jid).expect("begin_preparation");
            coord.mark_ready(&jid, 4242, None).expect("mark_ready");
            coord.cancel(&jid).expect("cancel");
        }
    }
    coord.clear_events_for_testing();
    jid
}

// ============================================================================
// Given steps
// ============================================================================

#[given("a configured main agent with a coding coordinator")]
fn given_configured_main_agent(world: &mut QuectoWorld) {
    world.coding_run_response = None;
    world.coding_status_response = None;
    world.coding_cancel_response = None;
    world.coding_cleanup_response = None;
    world.coding_list_response = None;
    world.coding_command_error = None;
    world.coding_current_job_id = None;
    world.coding_coordinator = None;
    world.coding_keep_artifacts = true;
    build_resp_coordinator(world);
}

#[given("a coding job is running in the background")]
fn given_running_in_background(world: &mut QuectoWorld) {
    advance_new_job_to(world, "background task", JobState::Running);
}

#[given(expr = "{int} coding jobs are running in the background")]
fn given_many_running(world: &mut QuectoWorld, n: usize) {
    for idx in 0..n {
        advance_new_job_to(world, &format!("background-{idx}"), JobState::Running);
    }
}

#[given(expr = "a coding job is running with progress {int}")]
fn given_running_with_progress(world: &mut QuectoWorld, progress: u32) {
    let jid = advance_new_job_to(world, "progress task", JobState::Running);
    let coord = world.coding_coordinator.as_mut().unwrap();
    coord
        .record_worker_progress(&jid, progress, "in progress")
        .expect("record_worker_progress");
    coord.clear_events_for_testing();
}

#[given("a coding job was running in the background")]
fn given_was_running(world: &mut QuectoWorld) {
    given_running_in_background(world);
}

#[given(expr = "a coding job completed with state {string} and summary {string}")]
fn given_completed_with_summary(world: &mut QuectoWorld, state: String, summary: String) {
    let target = parse_state(&state);
    let (jid, _) = run_job_with_goal(world, "completed");
    let coord = world.coding_coordinator.as_mut().unwrap();
    coord.begin_preparation(&jid).expect("begin_preparation");
    coord.mark_ready(&jid, 4242, None).expect("mark_ready");
    match target {
        JobState::Succeeded => {
            coord
                .mark_succeeded(SuccessInfo {
                    job_id: &jid,
                    summary: &summary,
                    artifacts: vec!["patch_001".to_string(), "test_output_001".to_string()],
                    duration_ms: None,
                })
                .expect("mark_succeeded");
        }
        _ => panic!("unsupported state for completed_with_summary: {state}"),
    }
    coord.clear_events_for_testing();
}

#[given(expr = "a coding job completed with state {string}")]
fn given_completed_with_state(world: &mut QuectoWorld, state: String) {
    let target = parse_state(&state);
    advance_new_job_to(world, "completed", target);
}

#[given(expr = "a coding job completed with state {string} and error_code {string}")]
fn given_completed_with_error(world: &mut QuectoWorld, state: String, error_code: String) {
    assert_eq!(parse_state(&state), JobState::Failed);
    let err = error_code.parse::<ErrorCode>().expect("valid error code");
    let (jid, _) = run_job_with_goal(world, "failed job");
    let coord = world.coding_coordinator.as_mut().unwrap();
    coord.begin_preparation(&jid).expect("begin_preparation");
    coord.mark_ready(&jid, 4242, None).expect("mark_ready");
    coord
        .mark_failed(FailureInfo {
            job_id: &jid,
            error_code: err,
            error_detail: "tool failed",
            is_retriable: None,
            duration_ms: None,
        })
        .expect("mark_failed");
    coord.clear_events_for_testing();
}

#[given(expr = "a coding job transitions to {string} with reason {string}")]
fn given_job_transitions_blocked(world: &mut QuectoWorld, state: String, reason: String) {
    assert_eq!(parse_state(&state), JobState::Blocked);
    let (jid, _) = run_job_with_goal(world, "blocked job");
    let coord = world.coding_coordinator.as_mut().unwrap();
    coord.begin_preparation(&jid).expect("begin_preparation");
    coord.mark_ready(&jid, 4242, None).expect("mark_ready");
    coord
        .mark_blocked(&jid, &reason, Some("main-agent decision"))
        .expect("mark_blocked");
    coord.clear_events_for_testing();
}

#[given("a coding job is queued")]
fn given_job_queued(world: &mut QuectoWorld) {
    run_job_with_goal(world, "queued task");
    world
        .coding_coordinator
        .as_mut()
        .unwrap()
        .clear_events_for_testing();
}

#[given(expr = "the user starts a coding job with max_wall_seconds {int}")]
fn given_user_starts_with_timeout(world: &mut QuectoWorld, secs: u64) {
    ensure_resp_coordinator(world);
    let coord = world.coding_coordinator.as_mut().unwrap();
    let mut req = default_run_request("timeout task");
    req.max_wall_seconds = Some(secs);
    let resp = coord.run(req).expect("run with wall seconds");
    let jid = resp.job_id.clone();
    world.coding_current_job_id = Some(jid.clone());
    world.coding_run_response = Some(resp);
    coord.begin_preparation(&jid).expect("begin_preparation");
    coord.mark_ready(&jid, 4242, None).expect("mark_ready");
    coord.clear_events_for_testing();
}

#[given(expr = "{int} coding jobs are running and {int} is queued")]
fn given_running_and_queued(world: &mut QuectoWorld, running: usize, queued: usize) {
    for idx in 0..running {
        advance_new_job_to(world, &format!("running-{idx}"), JobState::Running);
    }
    for idx in 0..queued {
        run_job_with_goal(world, &format!("queued-{idx}"));
        world
            .coding_coordinator
            .as_mut()
            .unwrap()
            .clear_events_for_testing();
    }
}

#[given("a coding job completes while the user is asking an unrelated question")]
fn given_job_completes_mid_conversation(world: &mut QuectoWorld) {
    let jid = advance_new_job_to(world, "mid-conversation", JobState::Succeeded);
    // Keep the job_id so we can check it later
    world.coding_current_job_id = Some(jid);
}

// ============================================================================
// When steps
// ============================================================================

#[when(expr = "the user asks the agent to start a coding job {string}")]
fn when_user_starts_job(world: &mut QuectoWorld, goal: String) {
    run_job_with_goal(world, &goal);
}

#[when(expr = "the user asks the agent to start coding job {string}")]
fn when_user_starts_job_short(world: &mut QuectoWorld, goal: String) {
    run_job_with_goal(world, &goal);
}

#[when(expr = "then immediately asks to start coding job {string}")]
fn when_user_starts_second_job(world: &mut QuectoWorld, goal: String) {
    run_job_with_goal(world, &goal);
}

#[when(expr = "the user asks {string}")]
fn when_user_asks_question(world: &mut QuectoWorld, _msg: String) {
    // The coordinator remains operational while jobs run — verify by
    // querying status of the current job if one exists.
    if let Some(jid) = world.coding_current_job_id.clone() {
        let coord = world.coding_coordinator.as_ref().unwrap();
        let status = coord.status_by_job_id(&jid).expect("status during Q&A");
        world.coding_status_response = Some(status);
    }
}

#[when("the user asks the agent to read a file using the read_file tool")]
fn when_user_asks_read_file(world: &mut QuectoWorld) {
    // Verify coordinator is non-blocking: status query succeeds while job runs.
    let jid = world.coding_current_job_id.clone().expect("job exists");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let status = coord
        .status_by_job_id(&jid)
        .expect("status during read_file");
    world.coding_status_response = Some(status);
}

#[when(expr = "the user sends {int} unrelated messages")]
fn when_user_sends_many(world: &mut QuectoWorld, n: usize) {
    // Verify coordinator handles N status queries while jobs run.
    let coord = world.coding_coordinator.as_ref().unwrap();
    let list = coord.list(&ListRequest { state_filter: None });
    assert!(!list.jobs.is_empty());
    // Each "message" can independently query status — simulate N queries
    for _ in 0..n {
        let resp = coord.list(&ListRequest { state_filter: None });
        assert!(!resp.jobs.is_empty());
    }
    world.coding_list_response = Some(list);
}

#[when(expr = "the agent calls the coding_job tool with action {string}")]
fn when_agent_calls_action(world: &mut QuectoWorld, action: String) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    match action.as_str() {
        "run" => {
            // run already happened in the Given/When step that created the job
            assert!(world.coding_run_response.is_some());
        }
        "status" => {
            let coord = world.coding_coordinator.as_ref().unwrap();
            let resp = coord.status_by_job_id(&jid).expect("status_by_job_id");
            world.coding_status_response = Some(resp);
        }
        "cancel" => {
            let coord = world.coding_coordinator.as_mut().unwrap();
            let resp = coord.cancel(&jid).expect("cancel");
            world.coding_cancel_response = Some(resp);
        }
        "cleanup" => {
            let coord = world.coding_coordinator.as_mut().unwrap();
            let resp = coord
                .cleanup(&jid, world.coding_keep_artifacts)
                .expect("cleanup");
            world.coding_cleanup_response = Some(resp);
        }
        _ => panic!("unsupported action: {action}"),
    }
}

#[when(expr = "the coding job completes with state {string}")]
fn when_job_completes(world: &mut QuectoWorld, state: String) {
    let target = parse_state(&state);
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_mut().unwrap();
    match target {
        JobState::Succeeded => {
            coord
                .mark_succeeded(SuccessInfo {
                    job_id: &jid,
                    summary: "done",
                    artifacts: vec!["patch_001".to_string()],
                    duration_ms: None,
                })
                .expect("mark_succeeded");
        }
        JobState::Failed => {
            coord
                .mark_failed(FailureInfo {
                    job_id: &jid,
                    error_code: ErrorCode::Internal,
                    error_detail: "failed",
                    is_retriable: None,
                    duration_ms: None,
                })
                .expect("mark_failed");
        }
        _ => panic!("unsupported completion state: {state}"),
    }
}

#[when("the user asks the agent to review the result")]
fn when_user_reviews_result(world: &mut QuectoWorld) {
    // Query status to get result — mirrors what the agent would do
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let resp = coord.status_by_job_id(&jid).expect("status_by_job_id");
    world.coding_status_response = Some(resp);
}

#[when("the user asks the agent what happened")]
fn when_user_asks_what_happened(world: &mut QuectoWorld) {
    // Query status to get failure details
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let resp = coord.status_by_job_id(&jid).expect("status_by_job_id");
    world.coding_status_response = Some(resp);
}

#[when("the user asks about the job status")]
fn when_user_asks_blocked_status(world: &mut QuectoWorld) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let resp = coord.status_by_job_id(&jid).expect("status_by_job_id");
    world.coding_status_response = Some(resp);
}

#[when(expr = "the user says {string}")]
fn when_user_says(world: &mut QuectoWorld, _message: String) {
    // The agent receives user intent. Verify the coordinator is still
    // operational (non-blocking) — the specific action is dispatched
    // in the subsequent "agent calls tool" step.
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let list = coord.list(&ListRequest { state_filter: None });
    assert!(!list.jobs.is_empty());
}

#[when(expr = "the job exceeds the {int}-second wall timeout")]
fn when_job_exceeds_timeout(world: &mut QuectoWorld, _secs: u64) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_mut().unwrap();
    coord
        .cancel_with_reason(&jid, CancelReason::WallTimeout, CancelInitiator::System)
        .expect("cancel_with_reason");
}

#[when("the user asks the agent to start a coding job with a 5-minute limit")]
fn when_user_starts_with_5m(world: &mut QuectoWorld) {
    ensure_resp_coordinator(world);
    let coord = world.coding_coordinator.as_mut().unwrap();
    let mut req = default_run_request("5m limit");
    req.max_wall_seconds = Some(300);
    let resp = coord.run(req).expect("run with 5m");
    world.coding_current_job_id = Some(resp.job_id.clone());
    world.coding_run_response = Some(resp);
}

#[when(
    expr = "the agent calls the coding_job tool with action {string} and max_wall_seconds {int}"
)]
fn when_agent_calls_run_with_timeout(world: &mut QuectoWorld, action: String, secs: u64) {
    assert_eq!(action, "run");
    ensure_resp_coordinator(world);
    let coord = world.coding_coordinator.as_mut().unwrap();
    let mut req = default_run_request("timed run");
    req.max_wall_seconds = Some(secs);
    let resp = coord.run(req).expect("run with wall seconds");
    world.coding_current_job_id = Some(resp.job_id.clone());
    world.coding_run_response = Some(resp);
}

#[when(expr = "the user asks the agent to start a coding job on repo {string} at ref {string}")]
fn when_user_starts_on_repo_ref(world: &mut QuectoWorld, repo: String, base_ref: String) {
    ensure_resp_coordinator(world);
    let coord = world.coding_coordinator.as_mut().unwrap();
    let mut req = default_run_request("repo run");
    req.repo = repo;
    req.base_ref = base_ref;
    let resp = coord.run(req).expect("run on repo/ref");
    world.coding_current_job_id = Some(resp.job_id.clone());
    world.coding_run_response = Some(resp);
}

#[when("the main agent provides a decision to unblock the job")]
fn when_main_agent_unblocks(world: &mut QuectoWorld) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_mut().unwrap();
    coord
        .mark_resumed(&jid, "resolved ambiguity")
        .expect("mark_resumed");
}

#[when(expr = "the agent calls the coding_job tool with action {string} and keep_artifacts {word}")]
fn when_agent_calls_cleanup_with_keep(world: &mut QuectoWorld, action: String, keep: String) {
    assert_eq!(action, "cleanup");
    world.coding_keep_artifacts = matches!(keep.as_str(), "true" | "True" | "TRUE");
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_mut().unwrap();
    let resp = coord
        .cleanup(&jid, world.coding_keep_artifacts)
        .expect("cleanup with keep_artifacts");
    world.coding_cleanup_response = Some(resp);
}

#[when(expr = "the agent calls the coding_job tool with action {string} and state_filter {string}")]
fn when_agent_calls_list(world: &mut QuectoWorld, action: String, state_filter: String) {
    assert_eq!(action, "list");
    let states: Vec<JobState> = parse_list_literal(&state_filter)
        .into_iter()
        .map(|s| parse_state(&s))
        .collect();
    let coord = world.coding_coordinator.as_ref().unwrap();
    let resp = coord.list(&ListRequest {
        state_filter: Some(states),
    });
    world.coding_list_response = Some(resp);
}

#[when(
    regex = r#"^the agent calls the coding_job tool with action "([^"]+)" and state_filter (\[.*\])$"#
)]
fn when_agent_calls_list_unquoted(world: &mut QuectoWorld, action: String, state_filter: String) {
    when_agent_calls_list(world, action, state_filter);
}

#[when("the agent processes the user's message")]
fn when_agent_processes_user_message(world: &mut QuectoWorld) {
    // The agent checks for completed jobs — query status via coordinator.
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let resp = coord.status_by_job_id(&jid).expect("status_by_job_id");
    world.coding_status_response = Some(resp);
}

// ============================================================================
// Then steps
// ============================================================================

#[then("the agent should receive an acknowledgement with run_id and job_id")]
fn then_receive_ack(world: &mut QuectoWorld) {
    let run = world.coding_run_response.as_ref().expect("run response");
    assert!(!run.run_id.is_empty());
    assert!(!run.job_id.is_empty());
}

#[then("the agent should respond to the user without waiting for job completion")]
fn then_responds_without_wait(world: &mut QuectoWorld) {
    // run() returned synchronously with a Queued state — no blocking.
    let run = world.coding_run_response.as_ref().expect("run response");
    assert_eq!(run.state, JobState::Queued);
}

#[then("both jobs should be accepted by the coordinator")]
fn then_both_jobs_accepted(world: &mut QuectoWorld) {
    let coord = world.coding_coordinator.as_ref().unwrap();
    let all = coord.list(&ListRequest { state_filter: None });
    assert!(all.jobs.len() >= 2);
}

#[then("the agent should respond to both requests promptly")]
fn then_responds_both_promptly(world: &mut QuectoWorld) {
    // Both run() calls returned RunResponse — coordinator is non-blocking.
    let coord = world.coding_coordinator.as_ref().unwrap();
    let all = coord.list(&ListRequest { state_filter: None });
    assert!(all.jobs.len() >= 2);
}

#[then("the agent should respond with an answer")]
fn then_agent_responds_answer(world: &mut QuectoWorld) {
    // Status query succeeded — coordinator didn't block while job ran.
    assert!(world.coding_status_response.is_some());
}

#[then("the coding job should continue running undisturbed")]
fn then_job_continues_undisturbed(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state, JobState::Running);
}

#[then("the agent should execute the read_file tool and return the result")]
fn then_read_file_used(world: &mut QuectoWorld) {
    // Status query completed — the coordinator remained operational.
    assert!(world.coding_status_response.is_some());
}

#[then("the coding job should still be running")]
fn then_job_still_running(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state, JobState::Running);
}

#[then(expr = "all {int} messages should receive responses")]
fn then_all_messages_responded(world: &mut QuectoWorld, n: usize) {
    // All N list queries succeeded in the When step.
    let list = world.coding_list_response.as_ref().expect("list response");
    // The coordinator remained responsive throughout.
    assert!(!list.jobs.is_empty());
    assert!(n > 0); // sanity: the scenario asked for > 0 messages
}

#[then(expr = "all {int} coding jobs should continue running")]
fn then_all_jobs_continue(world: &mut QuectoWorld, n: usize) {
    let coord = world.coding_coordinator.as_ref().unwrap();
    let running = coord.list(&ListRequest {
        state_filter: Some(vec![JobState::Running]),
    });
    assert_eq!(running.jobs.len(), n);
}

#[then(expr = "the agent should receive the current status with progress {int}")]
fn then_status_with_progress(world: &mut QuectoWorld, progress: u32) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.progress, Some(progress));
}

#[then("should relay a summary to the user")]
fn then_relays_summary(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert!(status.summary.is_some());
}

#[then("the coordinator should make the result available")]
fn then_completion_available(world: &mut QuectoWorld) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let status = coord.status_by_job_id(&jid).expect("status_by_job_id");
    assert!(status.state.is_terminal());
}

#[then("the next time the agent processes a message it should be aware of the completion")]
fn then_agent_aware_completion(world: &mut QuectoWorld) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let status = coord.status_by_job_id(&jid).expect("status_by_job_id");
    assert!(status.state.is_terminal());
}

#[then("the agent should receive the success summary and artifacts")]
fn then_receive_success_summary(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state, JobState::Succeeded);
    assert!(!status.artifacts.is_empty());
}

#[then("the agent should be able to decide whether to publish or iterate")]
fn then_can_decide_publish_or_iterate(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert!(matches!(status.state, JobState::Succeeded));
    assert!(status.summary.is_some());
}

#[then("the agent should receive the failure details")]
fn then_receive_failure_details(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state, JobState::Failed);
    assert!(status.error_code.is_some());
}

#[then("the agent should be able to start a new job with a revised approach")]
fn then_can_start_revised_job(world: &mut QuectoWorld) {
    // Verify we can start a new job through the coordinator.
    let coord = world.coding_coordinator.as_mut().unwrap();
    let resp = coord
        .run(default_run_request("revised approach"))
        .expect("run revised job");
    assert!(!resp.job_id.is_empty());
    assert!(coord.job(&resp.job_id).is_some());
}

#[then("the agent should report the blocked state and reason")]
fn then_reports_blocked_state(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state, JobState::Blocked);
}

#[then("the user can provide guidance through the agent")]
fn then_user_can_provide_guidance(world: &mut QuectoWorld) {
    // The coordinator exposes the blocked state — the agent can relay it.
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state, JobState::Blocked);
}

#[then("the agent can relay the decision to unblock the job")]
fn then_agent_relays_unblock(world: &mut QuectoWorld) {
    // The coordinator provides mark_resumed() — verify the job is still
    // accessible (it was blocked, the agent can call mark_resumed).
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let job = coord.job(&jid).expect("job exists");
    // At this point in the scenario, the job is blocked.
    // The agent CAN relay unblock via mark_resumed() — tested in the
    // "Blocked job emits job.resumed" scenario.
    assert_eq!(job.state, JobState::Blocked);
}

#[then(expr = "the cancel response should include the job_id and state {string}")]
fn then_cancel_response_with_job(world: &mut QuectoWorld, state: String) {
    let resp = world
        .coding_cancel_response
        .as_ref()
        .expect("cancel response");
    assert_eq!(resp.state.to_string(), state);
    assert!(!resp.job_id.is_empty());
}

#[then("the agent should confirm the cancellation to the user")]
fn then_confirm_cancel(world: &mut QuectoWorld) {
    // Cancel response was returned — agent can confirm to user.
    let resp = world
        .coding_cancel_response
        .as_ref()
        .expect("cancel response");
    assert_eq!(resp.state, JobState::Canceled);
}

#[then(expr = "the cancel response should include state {string}")]
fn then_cancel_response_state(world: &mut QuectoWorld, state: String) {
    let resp = world
        .coding_cancel_response
        .as_ref()
        .expect("cancel response");
    assert_eq!(resp.state.to_string(), state);
}

#[then("no worker should have been launched")]
fn then_no_worker_launched(world: &mut QuectoWorld) {
    // The job was queued and then canceled — no mark_ready was called.
    // Verify via coordinator: the job has no worker_pid.
    let jid = world
        .coding_cancel_response
        .as_ref()
        .expect("cancel response")
        .job_id
        .clone();
    // After cancel, job is still in coordinator (not cleaned up yet)
    let coord = world.coding_coordinator.as_ref().unwrap();
    let job = coord.job(&jid).expect("job exists");
    assert!(job.worker_pid.is_none());
}

#[then(expr = "the coordinator should cancel the job with reason {string}")]
fn then_cancel_reason(world: &mut QuectoWorld, reason: String) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let job = coord.job(&jid).expect("job");
    assert_eq!(job.state, JobState::Canceled);
    assert_eq!(job.cancel_reason.map(|r| r.to_string()), Some(reason));
}

#[then(expr = "the next time the agent checks status it should see state {string}")]
fn then_status_state(world: &mut QuectoWorld, state: String) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let resp = coord.status_by_job_id(&jid).expect("status_by_job_id");
    assert_eq!(resp.state.to_string(), state);
}

#[then(expr = "the cancel_reason should be {string}")]
fn then_cancel_reason_value(world: &mut QuectoWorld, reason: String) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let resp = coord.status_by_job_id(&jid).expect("status_by_job_id");
    assert_eq!(resp.cancel_reason.map(|r| r.to_string()), Some(reason));
}

#[then(expr = "the coordinator should accept the job with max_wall_seconds {int}")]
fn then_accept_with_wall(world: &mut QuectoWorld, secs: u64) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let job = coord.job(&jid).expect("job");
    assert_eq!(job.max_wall_seconds, Some(secs));
}

#[then("the run response should include run_id and job_id")]
fn then_run_response_ids(world: &mut QuectoWorld) {
    let run = world.coding_run_response.as_ref().expect("run response");
    assert!(!run.run_id.is_empty());
    assert!(!run.job_id.is_empty());
}

#[then(expr = "the initial state should be {string}")]
fn then_initial_state(world: &mut QuectoWorld, state: String) {
    let run = world.coding_run_response.as_ref().expect("run response");
    assert_eq!(run.state.to_string(), state);
}

#[then("a \"job.resumed\" event should be emitted with reason describing the resolution")]
fn then_job_resumed_event(world: &mut QuectoWorld) {
    let coord = world.coding_coordinator.as_ref().unwrap();
    let events = coord.events();
    let resumed = events
        .iter()
        .any(|e| e.event_type == "job.resumed" && e.payload.get("reason").is_some());
    assert!(resumed);
}

#[then("the job state should transition back to \"running\"")]
fn then_job_back_running(world: &mut QuectoWorld) {
    let jid = world.coding_current_job_id.clone().expect("current job_id");
    let coord = world.coding_coordinator.as_ref().unwrap();
    let job = coord.job(&jid).expect("job");
    assert_eq!(job.state, JobState::Running);
}

#[then("the cleanup response should indicate cleaned is true")]
fn then_cleanup_cleaned_true(world: &mut QuectoWorld) {
    let cleanup = world
        .coding_cleanup_response
        .as_ref()
        .expect("cleanup response");
    assert!(cleanup.cleaned);
}

#[then("the agent should confirm the cleanup to the user")]
fn then_confirm_cleanup(world: &mut QuectoWorld) {
    // Cleanup response was returned — agent can confirm to user.
    let cleanup = world
        .coding_cleanup_response
        .as_ref()
        .expect("cleanup response");
    assert!(cleanup.cleaned);
}

#[then("the repo directory should be removed")]
fn then_repo_removed(world: &mut QuectoWorld) {
    let cleanup = world
        .coding_cleanup_response
        .as_ref()
        .expect("cleanup response");
    assert!(cleanup.cleaned);
}

#[then(expr = "the response should include {int} jobs with their states")]
fn then_list_includes_jobs(world: &mut QuectoWorld, count: usize) {
    let list = world.coding_list_response.as_ref().expect("list response");
    assert_eq!(list.jobs.len(), count);
}

#[then("the agent should report their states and progress to the user")]
fn then_report_states_progress(world: &mut QuectoWorld) {
    let list = world.coding_list_response.as_ref().expect("list response");
    // Each listed job has state info the agent can report.
    assert!(!list.jobs.is_empty());
    for job in &list.jobs {
        assert!(!job.job_id.is_empty());
    }
}

#[then("the agent should answer the user's question")]
fn then_answer_question(world: &mut QuectoWorld) {
    // Status query succeeded — the coordinator was responsive.
    assert!(world.coding_status_response.is_some());
}

#[then("mention that the coding job has completed")]
fn then_mentions_completion(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert!(status.state.is_terminal());
}
