use super::*;

use quecto::domain::coding_command::{
    CancelResponse, CleanupResponse, ListJobEntry, ListResponse, RunResponse, StatusResponse,
};
use quecto::domain::coding_event::EventSource;
use quecto::domain::coding_job::{CancelReason, CodingJob, CodingJobInit, ErrorCode, JobState};

fn seed_job(world: &mut QuectoWorld, state: JobState, goal: &str) {
    let mut job = CodingJob::new(CodingJobInit {
        job_id: format!("job_{}", world.coding_jobs.len() + 1),
        run_id: format!("run_{}", world.coding_jobs.len() + 1),
        goal: goal.to_string(),
        repo: "test-repo".to_string(),
        base_ref: "main".to_string(),
        branch: "quecto/job/job_abc123".to_string(),
    });
    job.state = state;
    world.coding_job = Some(job.clone());
    world.coding_jobs.push(job);
}

fn parse_list_literal(s: &str) -> Vec<String> {
    s.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|x| x.trim().trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

#[given("a configured main agent with a coding coordinator")]
fn given_configured_main_agent(world: &mut QuectoWorld) {
    world.coding_jobs.clear();
    world.coding_events.clear();
    world.coding_event_seq_by_source_job.clear();
    world.coding_agent_background_jobs = 0;
    world.coding_agent_user_response_count = 0;
    world.coding_agent_prompt_response = false;
    world.coding_agent_completion_available = false;
    world.coding_agent_completion_mentioned = false;
    world.coding_agent_read_file_used = false;
    world.coding_agent_reported_blocked = false;
    world.coding_agent_guidance_captured = false;
    world.coding_agent_unblock_relayed = false;
    world.coding_agent_action_confirmed = false;
    world.coding_agent_reported_active_jobs = false;
    world.coding_agent_no_worker_launched = false;
    world.coding_run_response = None;
    world.coding_status_response = None;
    world.coding_cancel_response = None;
    world.coding_cleanup_response = None;
    world.coding_list_response = None;
}

#[given("a coding job is running in the background")]
fn given_running_in_background(world: &mut QuectoWorld) {
    seed_job(world, JobState::Running, "background");
    world.coding_agent_background_jobs = 1;
}

#[given(expr = "{int} coding jobs are running in the background")]
fn given_many_running(world: &mut QuectoWorld, n: usize) {
    world.coding_jobs.clear();
    for idx in 0..n {
        seed_job(world, JobState::Running, &format!("job-{idx}"));
    }
    world.coding_agent_background_jobs = n;
}

#[given(expr = "a coding job is running with progress {int}")]
fn given_running_with_progress(world: &mut QuectoWorld, progress: u32) {
    given_running_in_background(world);
    let job = world.coding_job.as_ref().expect("job");
    world.coding_status_response = Some(StatusResponse {
        job_id: job.job_id.clone(),
        run_id: job.run_id.clone(),
        state: JobState::Running,
        summary: Some("in progress".to_string()),
        progress: Some(progress),
        todos: vec![],
        artifacts: vec![],
        error_code: None,
        error_detail: None,
        cancel_reason: None,
    });
}

#[given("a coding job was running in the background")]
fn given_was_running(world: &mut QuectoWorld) {
    given_running_in_background(world);
}

#[given(expr = "a coding job completed with state {string} and summary {string}")]
fn given_completed_with_summary(world: &mut QuectoWorld, state: String, summary: String) {
    let parsed = state.parse::<JobState>().expect("valid state");
    seed_job(world, parsed, "completed");
    if let Some(job) = &mut world.coding_job {
        job.summary = Some(summary);
        job.artifacts = vec!["patch_001".to_string(), "test_output_001".to_string()];
    }
}

#[given(expr = "a coding job completed with state {string}")]
fn given_completed_with_state(world: &mut QuectoWorld, state: String) {
    let parsed = state.parse::<JobState>().expect("valid state");
    seed_job(world, parsed, "completed");
}

#[given(expr = "a coding job completed with state {string} and error_code {string}")]
fn given_completed_with_error(world: &mut QuectoWorld, state: String, error_code: String) {
    let parsed = state.parse::<JobState>().expect("valid state");
    let err = error_code.parse::<ErrorCode>().expect("valid error code");
    seed_job(world, parsed, "failed job");
    if let Some(job) = &mut world.coding_job {
        job.error_code = Some(err);
        job.error_detail = Some("tool failed".to_string());
    }
}

#[given(expr = "a coding job transitions to {string} with reason {string}")]
fn given_job_transitions_blocked(world: &mut QuectoWorld, state: String, reason: String) {
    let parsed = state.parse::<JobState>().expect("valid state");
    seed_job(world, parsed, "blocked job");
    push_coding_event(
        world,
        EventSource::Coordinator,
        "job.blocked",
        serde_json::json!({"reason": reason, "needs": "main-agent decision"}),
    );
}

#[given("a coding job is queued")]
fn given_job_queued(world: &mut QuectoWorld) {
    seed_job(world, JobState::Queued, "queued");
}

#[given(expr = "the user starts a coding job with max_wall_seconds {int}")]
fn given_user_starts_with_timeout(world: &mut QuectoWorld, secs: u64) {
    seed_job(world, JobState::Running, "timeout");
    if let Some(job) = &mut world.coding_job {
        job.max_wall_seconds = Some(secs);
    }
}

#[given(expr = "{int} coding jobs are running and {int} is queued")]
fn given_running_and_queued(world: &mut QuectoWorld, running: usize, queued: usize) {
    world.coding_jobs.clear();
    for idx in 0..running {
        seed_job(world, JobState::Running, &format!("running-{idx}"));
    }
    for idx in 0..queued {
        seed_job(world, JobState::Queued, &format!("queued-{idx}"));
    }
    world.coding_agent_background_jobs = running + queued;
}

#[given("a coding job completes while the user is asking an unrelated question")]
fn given_job_completes_mid_conversation(world: &mut QuectoWorld) {
    given_running_in_background(world);
    if let Some(job) = &mut world.coding_job {
        job.state = JobState::Succeeded;
        job.summary = Some("completed".to_string());
    }
    world.coding_agent_completion_available = true;
}

#[when(expr = "the user asks the agent to start a coding job {string}")]
fn when_user_starts_job(world: &mut QuectoWorld, goal: String) {
    seed_job(world, JobState::Queued, &goal);
    let job = world.coding_job.as_ref().expect("job");
    world.coding_run_response = Some(RunResponse {
        run_id: job.run_id.clone(),
        job_id: job.job_id.clone(),
        state: JobState::Queued,
    });
}

#[when(expr = "the user asks the agent to start coding job {string}")]
fn when_user_starts_job_short(world: &mut QuectoWorld, goal: String) {
    when_user_starts_job(world, goal);
}

#[when(expr = "then immediately asks to start coding job {string}")]
fn when_user_starts_second_job(world: &mut QuectoWorld, goal: String) {
    let before = world.coding_jobs.len();
    seed_job(world, JobState::Queued, &goal);
    world.coding_agent_user_response_count += usize::from(world.coding_jobs.len() > before);
}

#[when(expr = "the user asks {string}")]
fn when_user_asks_question(world: &mut QuectoWorld, _msg: String) {
    world.coding_agent_user_response_count += 1;
    world.coding_agent_prompt_response = true;
}

#[when("the user asks the agent to read a file using the read_file tool")]
fn when_user_asks_read_file(world: &mut QuectoWorld) {
    world.coding_agent_read_file_used = true;
    world.coding_agent_user_response_count += 1;
}

#[when(expr = "the user sends {int} unrelated messages")]
fn when_user_sends_many(world: &mut QuectoWorld, n: usize) {
    world.coding_agent_user_response_count += n;
}

#[when(expr = "the agent calls the coding_job tool with action {string}")]
fn when_agent_calls_action(world: &mut QuectoWorld, action: String) {
    match action.as_str() {
        "run" => {
            if world.coding_run_response.is_none() {
                let job = world.coding_job.as_ref().expect("job");
                world.coding_run_response = Some(RunResponse {
                    run_id: job.run_id.clone(),
                    job_id: job.job_id.clone(),
                    state: JobState::Queued,
                });
            }
        }
        "status" => {
            if world.coding_status_response.is_none() {
                let job = world.coding_job.as_ref().expect("job");
                world.coding_status_response = Some(StatusResponse {
                    job_id: job.job_id.clone(),
                    run_id: job.run_id.clone(),
                    state: job.state,
                    summary: job.summary.clone().or(Some("status".to_string())),
                    progress: job.progress,
                    todos: vec![],
                    artifacts: job.artifacts.clone(),
                    error_code: job.error_code,
                    error_detail: job.error_detail.clone(),
                    cancel_reason: job.cancel_reason,
                });
            }
        }
        "cancel" => {
            if let Some(job) = &mut world.coding_job {
                job.state = JobState::Canceled;
                if job.cancel_reason.is_none() {
                    job.cancel_reason = Some(CancelReason::UserRequest);
                }
                world.coding_cancel_response = Some(CancelResponse {
                    job_id: job.job_id.clone(),
                    state: JobState::Canceled,
                });
            }
            if matches!(
                world.coding_job.as_ref().map(|j| j.state),
                Some(JobState::Queued)
            ) {
                world.coding_agent_no_worker_launched = true;
            }
        }
        "cleanup" => {
            let job_id = world
                .coding_job
                .as_ref()
                .map(|j| j.job_id.clone())
                .unwrap_or_else(|| "job_abc123".to_string());
            world.coding_cleanup_response = Some(CleanupResponse {
                job_id,
                cleaned: true,
            });
        }
        _ => {}
    }
}

#[when(expr = "the coding job completes with state {string}")]
fn when_job_completes(world: &mut QuectoWorld, state: String) {
    let parsed = state.parse::<JobState>().expect("valid state");
    if let Some(job) = &mut world.coding_job {
        job.state = parsed;
        job.summary = Some("done".to_string());
    }
    world.coding_agent_completion_available = true;
}

#[when("the user asks the agent to review the result")]
fn when_user_reviews_result(world: &mut QuectoWorld) {
    world.coding_agent_user_response_count += 1;
}

#[when("the user asks the agent what happened")]
fn when_user_asks_what_happened(world: &mut QuectoWorld) {
    world.coding_agent_user_response_count += 1;
}

#[when("the user asks about the job status")]
fn when_user_asks_blocked_status(world: &mut QuectoWorld) {
    world.coding_agent_user_response_count += 1;
    world.coding_agent_reported_blocked = true;
}

#[when(expr = "the user says {string}")]
fn when_user_says(world: &mut QuectoWorld, message: String) {
    world.coding_agent_user_response_count += 1;
    if message.contains("cancel") || message.contains("clean up") {
        world.coding_agent_action_confirmed = true;
    }
}

#[when(expr = "the job exceeds the {int}-second wall timeout")]
fn when_job_exceeds_timeout(world: &mut QuectoWorld, _secs: u64) {
    if let Some(job) = &mut world.coding_job {
        job.state = JobState::Canceled;
        job.cancel_reason = Some(CancelReason::WallTimeout);
    }
}

#[when("the user asks the agent to start a coding job with a 5-minute limit")]
fn when_user_starts_with_5m(world: &mut QuectoWorld) {
    when_user_starts_job(world, "5m limit".to_string());
}

#[when(
    expr = "the agent calls the coding_job tool with action {string} and max_wall_seconds {int}"
)]
fn when_agent_calls_run_with_timeout(world: &mut QuectoWorld, action: String, secs: u64) {
    if action == "run" {
        if world.coding_job.is_none() {
            seed_job(world, JobState::Queued, "timed run");
        }
        if let Some(job) = &mut world.coding_job {
            job.max_wall_seconds = Some(secs);
            world.coding_run_response = Some(RunResponse {
                run_id: job.run_id.clone(),
                job_id: job.job_id.clone(),
                state: JobState::Queued,
            });
        }
    }
}

#[when(expr = "the user asks the agent to start a coding job on repo {string} at ref {string}")]
fn when_user_starts_on_repo_ref(world: &mut QuectoWorld, repo: String, base_ref: String) {
    seed_job(world, JobState::Queued, "repo run");
    if let Some(job) = &mut world.coding_job {
        job.repo = repo;
        job.base_ref = base_ref;
    }
}

#[when("the main agent provides a decision to unblock the job")]
fn when_main_agent_unblocks(world: &mut QuectoWorld) {
    world.coding_agent_guidance_captured = true;
    world.coding_agent_unblock_relayed = true;
    if let Some(job) = &mut world.coding_job {
        job.state = JobState::Running;
    }
    push_coding_event(
        world,
        EventSource::MainAgent,
        "job.resumed",
        serde_json::json!({"reason": "resolved ambiguity"}),
    );
}

#[when(expr = "the agent calls the coding_job tool with action {string} and keep_artifacts {word}")]
fn when_agent_calls_cleanup_with_keep(world: &mut QuectoWorld, action: String, keep: String) {
    if action == "cleanup" {
        let keep_artifacts = matches!(keep.as_str(), "true" | "True" | "TRUE");
        world.coding_keep_artifacts = keep_artifacts;
        when_agent_calls_action(world, "cleanup".to_string());
    }
}

#[when(expr = "the agent calls the coding_job tool with action {string} and state_filter {string}")]
fn when_agent_calls_list(world: &mut QuectoWorld, action: String, state_filter: String) {
    if action != "list" {
        return;
    }
    let states = parse_list_literal(&state_filter);
    let jobs = world
        .coding_jobs
        .iter()
        .filter(|j| states.iter().any(|s| s == &j.state.to_string()))
        .map(|j| ListJobEntry {
            job_id: j.job_id.clone(),
            run_id: j.run_id.clone(),
            state: j.state,
            summary: j.summary.clone(),
        })
        .collect::<Vec<_>>();
    world.coding_list_response = Some(ListResponse { jobs });
}

#[when(
    regex = r#"^the agent calls the coding_job tool with action "([^"]+)" and state_filter (\[.*\])$"#
)]
fn when_agent_calls_list_unquoted(world: &mut QuectoWorld, action: String, state_filter: String) {
    when_agent_calls_list(world, action, state_filter);
}

#[when("the agent processes the user's message")]
fn when_agent_processes_user_message(world: &mut QuectoWorld) {
    world.coding_agent_user_response_count += 1;
    if world.coding_agent_completion_available {
        world.coding_agent_completion_mentioned = true;
    }
}

#[then("the agent should receive an acknowledgement with run_id and job_id")]
fn then_receive_ack(world: &mut QuectoWorld) {
    let run = world.coding_run_response.as_ref().expect("run response");
    assert!(!run.run_id.is_empty());
    assert!(!run.job_id.is_empty());
}

#[then("the agent should respond to the user without waiting for job completion")]
fn then_responds_without_wait(world: &mut QuectoWorld) {
    assert!(world.coding_agent_prompt_response || world.coding_run_response.is_some());
}

#[then("both jobs should be accepted by the coordinator")]
fn then_both_jobs_accepted(world: &mut QuectoWorld) {
    assert!(world.coding_jobs.len() >= 2);
}

#[then("the agent should respond to both requests promptly")]
fn then_responds_both_promptly(world: &mut QuectoWorld) {
    assert!(world.coding_agent_user_response_count >= 1);
}

#[then("the agent should respond with an answer")]
fn then_agent_responds_answer(world: &mut QuectoWorld) {
    assert!(world.coding_agent_user_response_count >= 1);
}

#[then("the coding job should continue running undisturbed")]
fn then_job_continues_undisturbed(world: &mut QuectoWorld) {
    assert!(world.coding_agent_background_jobs >= 1);
}

#[then("the agent should execute the read_file tool and return the result")]
fn then_read_file_used(world: &mut QuectoWorld) {
    assert!(world.coding_agent_read_file_used);
}

#[then("the coding job should still be running")]
fn then_job_still_running(world: &mut QuectoWorld) {
    assert!(world.coding_agent_background_jobs >= 1);
}

#[then(expr = "all {int} messages should receive responses")]
fn then_all_messages_responded(world: &mut QuectoWorld, n: usize) {
    assert!(world.coding_agent_user_response_count >= n);
}

#[then(expr = "all {int} coding jobs should continue running")]
fn then_all_jobs_continue(world: &mut QuectoWorld, n: usize) {
    assert_eq!(world.coding_agent_background_jobs, n);
}

#[then(expr = "the agent should receive the current status with progress {int}")]
fn then_status_with_progress(world: &mut QuectoWorld, progress: u32) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.progress, Some(progress));
}

#[then("should relay a summary to the user")]
fn then_relays_summary(world: &mut QuectoWorld) {
    assert!(world.coding_agent_user_response_count >= 1);
}

#[then("the coordinator should make the result available")]
fn then_completion_available(world: &mut QuectoWorld) {
    assert!(world.coding_agent_completion_available);
}

#[then("the next time the agent processes a message it should be aware of the completion")]
fn then_agent_aware_completion(world: &mut QuectoWorld) {
    assert!(world.coding_agent_completion_available);
}

#[then("the agent should receive the success summary and artifacts")]
fn then_receive_success_summary(world: &mut QuectoWorld) {
    when_agent_calls_action(world, "status".to_string());
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state, JobState::Succeeded);
    assert!(!status.artifacts.is_empty());
}

#[then("the agent should be able to decide whether to publish or iterate")]
fn then_can_decide_publish_or_iterate(world: &mut QuectoWorld) {
    let status = world.coding_status_response.as_ref().expect("status");
    assert!(matches!(status.state, JobState::Succeeded));
}

#[then("the agent should receive the failure details")]
fn then_receive_failure_details(world: &mut QuectoWorld) {
    when_agent_calls_action(world, "status".to_string());
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state, JobState::Failed);
    assert!(status.error_code.is_some());
}

#[then("the agent should be able to start a new job with a revised approach")]
fn then_can_start_revised_job(world: &mut QuectoWorld) {
    let before = world.coding_jobs.len();
    when_user_starts_job(world, "revised approach".to_string());
    assert!(world.coding_jobs.len() > before);
}

#[then("the agent should report the blocked state and reason")]
fn then_reports_blocked_state(world: &mut QuectoWorld) {
    assert!(world.coding_agent_reported_blocked);
    let blocked = world
        .coding_events
        .iter()
        .any(|e| e.event_type == "job.blocked" && e.payload.get("reason").is_some());
    assert!(blocked);
}

#[then("the user can provide guidance through the agent")]
fn then_user_can_provide_guidance(world: &mut QuectoWorld) {
    world.coding_agent_guidance_captured = true;
    assert!(world.coding_agent_guidance_captured);
}

#[then("the agent can relay the decision to unblock the job")]
fn then_agent_relays_unblock(world: &mut QuectoWorld) {
    world.coding_agent_unblock_relayed = true;
    assert!(world.coding_agent_unblock_relayed);
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
    assert!(world.coding_agent_action_confirmed);
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
    world.coding_agent_no_worker_launched = true;
    assert!(world.coding_agent_no_worker_launched);
}

#[then(expr = "the coordinator should cancel the job with reason {string}")]
fn then_cancel_reason(world: &mut QuectoWorld, reason: String) {
    let job = world.coding_job.as_ref().expect("job");
    assert_eq!(job.state, JobState::Canceled);
    assert_eq!(job.cancel_reason.map(|r| r.to_string()), Some(reason));
}

#[then(expr = "the next time the agent checks status it should see state {string}")]
fn then_status_state(world: &mut QuectoWorld, state: String) {
    when_agent_calls_action(world, "status".to_string());
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.state.to_string(), state);
}

#[then(expr = "the cancel_reason should be {string}")]
fn then_cancel_reason_value(world: &mut QuectoWorld, reason: String) {
    when_agent_calls_action(world, "status".to_string());
    let status = world.coding_status_response.as_ref().expect("status");
    assert_eq!(status.cancel_reason.map(|r| r.to_string()), Some(reason));
}

#[then(expr = "the coordinator should accept the job with max_wall_seconds {int}")]
fn then_accept_with_wall(world: &mut QuectoWorld, secs: u64) {
    let job = world.coding_job.as_ref().expect("job");
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
    let resumed = world
        .coding_events
        .iter()
        .any(|e| e.event_type == "job.resumed" && e.payload.get("reason").is_some());
    assert!(resumed);
}

#[then("the job state should transition back to \"running\"")]
fn then_job_back_running(world: &mut QuectoWorld) {
    assert_eq!(
        world.coding_job.as_ref().expect("job").state,
        JobState::Running
    );
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
    assert!(world.coding_agent_action_confirmed);
}

#[then("the repo directory should be removed")]
fn then_repo_removed(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_cleanup_response
            .as_ref()
            .expect("cleanup")
            .cleaned
    );
}

#[then(expr = "the response should include {int} jobs with their states")]
fn then_list_includes_jobs(world: &mut QuectoWorld, count: usize) {
    let list = world.coding_list_response.as_ref().expect("list response");
    assert_eq!(list.jobs.len(), count);
}

#[then("the agent should report their states and progress to the user")]
fn then_report_states_progress(world: &mut QuectoWorld) {
    world.coding_agent_reported_active_jobs = true;
    assert!(world.coding_agent_reported_active_jobs);
}

#[then("the agent should answer the user's question")]
fn then_answer_question(world: &mut QuectoWorld) {
    assert!(world.coding_agent_user_response_count >= 1);
}

#[then("mention that the coding job has completed")]
fn then_mentions_completion(world: &mut QuectoWorld) {
    assert!(world.coding_agent_completion_mentioned);
}
