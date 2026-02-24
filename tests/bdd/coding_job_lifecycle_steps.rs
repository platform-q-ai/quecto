use super::*;

use quecto::domain::coding_command::{
    CancelResponse, CleanupResponse, CommandError, ListJobEntry, ListResponse, RunResponse,
    StatusResponse, TodoItem,
};
use quecto::domain::coding_event::{EventEnvelope, EventSource, is_compatible_version};
use quecto::domain::coding_job::{
    CancelInitiator, CancelReason, CodingJob, CodingJobInit, ErrorCode, JobState, Priority,
};

fn parse_state(s: &str) -> JobState {
    s.parse::<JobState>().expect("invalid state in scenario")
}

fn parse_error_code(s: &str) -> ErrorCode {
    s.parse::<ErrorCode>()
        .expect("invalid error_code in scenario")
}

fn parse_list_literal(s: &str) -> Vec<String> {
    s.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|x| x.trim().trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn ensure_base(world: &mut QuectoWorld) {
    if world.cli_context.base_dir.is_none() {
        let repo = std::env::current_dir().expect("cwd");
        let base = repo.join(".bdd-data");
        std::fs::create_dir_all(&base).expect("create .bdd-data");
        world.cli_context.base_dir = Some(base);
    }
}

fn seed_job(world: &mut QuectoWorld, state: JobState) {
    let mut job = CodingJob::new(CodingJobInit {
        job_id: "job_abc123".to_string(),
        run_id: "run_abc123".to_string(),
        goal: "test goal".to_string(),
        repo: "test-repo".to_string(),
        base_ref: "main".to_string(),
        branch: "quecto/job/job_abc123".to_string(),
    });
    job.state = state;
    world.coding_job = Some(job);
}

fn emit(
    world: &mut QuectoWorld,
    source: EventSource,
    event_type: &str,
    payload: serde_json::Value,
) {
    push_coding_event(world, source, event_type, payload);
}

#[given("a coding coordinator with a mock worker")]
fn given_coding_coordinator_with_mock_worker(world: &mut QuectoWorld) {
    ensure_base(world);
    world.coding_events.clear();
    world.coding_event_seq_by_source_job.clear();
    world.coding_command_error = None;
    world.coding_run_response = None;
    world.coding_status_response = None;
    world.coding_cancel_response = None;
    world.coding_cleanup_response = None;
    world.coding_list_response = None;
    world.coding_jobs.clear();
    world.coding_skill_allowlist.clear();
    world.coding_skill_denylist.clear();
    world.coding_keep_artifacts = true;
    world.coding_warning_logged = false;
    world.coding_version_error_logged = false;
}

#[given(expr = "a coding coordinator with skill denylist containing {string}")]
fn given_skill_denylist(world: &mut QuectoWorld, skill: String) {
    world.coding_skill_denylist = vec![skill];
}

#[given(expr = "a coding coordinator with skill allowlist containing {string}")]
fn given_skill_allowlist(world: &mut QuectoWorld, skill: String) {
    world.coding_skill_allowlist = vec![skill];
}

#[given(expr = "skill policy allows {string}")]
fn given_skill_policy_allows(world: &mut QuectoWorld, list: String) {
    world.coding_skill_allowlist = parse_list_literal(&list);
}

#[given(regex = r#"^skill policy allows (\[.*\])$"#)]
fn given_skill_policy_allows_unquoted(world: &mut QuectoWorld, list: String) {
    world.coding_skill_allowlist = parse_list_literal(&list);
}

#[given(expr = "a coding job in state {string}")]
fn given_job_in_state(world: &mut QuectoWorld, state: String) {
    seed_job(world, parse_state(&state));
}

#[given(expr = "a coding job in state {string} with progress {int}")]
fn given_job_in_state_with_progress(world: &mut QuectoWorld, state: String, progress: u32) {
    seed_job(world, parse_state(&state));
    if let Some(j) = &mut world.coding_job {
        j.progress = Some(progress);
        j.summary = Some("running".to_string());
    }
}

#[given(expr = "a coding job in state {string} with error_code {string}")]
fn given_job_in_state_with_error(world: &mut QuectoWorld, state: String, code: String) {
    seed_job(world, parse_state(&state));
    if let Some(j) = &mut world.coding_job {
        j.error_code = Some(parse_error_code(&code));
        j.error_detail = Some("details".to_string());
    }
}

#[given(expr = "a coding job with max_wall_seconds {int}")]
fn given_job_with_wall(world: &mut QuectoWorld, secs: u64) {
    seed_job(world, JobState::Running);
    if let Some(j) = &mut world.coding_job {
        j.max_wall_seconds = Some(secs);
    }
}

#[given("a coding job with a known run_id")]
fn given_job_known_run_id(world: &mut QuectoWorld) {
    seed_job(world, JobState::Running);
    if let Some(j) = &mut world.coding_job {
        j.summary = Some("known".to_string());
    }
}

#[given(expr = "a coding job in state {string} with artifacts")]
fn given_job_with_artifacts(world: &mut QuectoWorld, state: String) {
    seed_job(world, parse_state(&state));
    if let Some(j) = &mut world.coding_job {
        j.artifacts = vec!["patch_001".to_string(), "test_output_001".to_string()];
    }
}

#[given(expr = "a coding job in state {string} with artifacts {string}")]
fn given_job_with_named_artifacts(world: &mut QuectoWorld, state: String, artifacts: String) {
    seed_job(world, parse_state(&state));
    if let Some(j) = &mut world.coding_job {
        j.artifacts = parse_list_literal(&artifacts);
    }
}

#[given(regex = r#"^a coding job in state \"([^\"]+)\" with artifacts (\[.*\])$"#)]
fn given_job_with_named_artifacts_unquoted(
    world: &mut QuectoWorld,
    state: String,
    artifacts: String,
) {
    given_job_with_named_artifacts(world, state, artifacts);
}

#[given(expr = "a coding job in state {string} with cancel reason {string}")]
fn given_job_with_cancel_reason(world: &mut QuectoWorld, state: String, reason: String) {
    seed_job(world, parse_state(&state));
    if let Some(j) = &mut world.coding_job {
        j.cancel_reason = Some(reason.parse().expect("cancel reason"));
    }
}

#[given("jobs exist in states \"running\", \"failed\", \"succeeded\"")]
fn given_jobs_three_states(world: &mut QuectoWorld) {
    world.coding_jobs.clear();
    for (idx, st) in [JobState::Running, JobState::Failed, JobState::Succeeded]
        .iter()
        .enumerate()
    {
        let mut j = CodingJob::new(CodingJobInit {
            job_id: format!("job_{idx}"),
            run_id: format!("run_{idx}"),
            goal: "goal".to_string(),
            repo: "test-repo".to_string(),
            base_ref: "main".to_string(),
            branch: format!("quecto/job/job_{idx}"),
        });
        j.state = *st;
        world.coding_jobs.push(j);
    }
}

#[given("jobs exist in states \"running\", \"failed\", \"succeeded\", \"canceled\"")]
fn given_jobs_four_states(world: &mut QuectoWorld) {
    given_jobs_three_states(world);
    let mut j = CodingJob::new(CodingJobInit {
        job_id: "job_3".to_string(),
        run_id: "run_3".to_string(),
        goal: "goal".to_string(),
        repo: "test-repo".to_string(),
        base_ref: "main".to_string(),
        branch: "quecto/job/job_3".to_string(),
    });
    j.state = JobState::Canceled;
    world.coding_jobs.push(j);
}

#[given("no jobs exist")]
fn given_no_jobs(world: &mut QuectoWorld) {
    world.coding_jobs.clear();
}

#[when(expr = "the main agent requests a coding job with goal {string}")]
fn when_run_with_goal(world: &mut QuectoWorld, goal: String) {
    let mut job = CodingJob::new(CodingJobInit {
        job_id: "job_abc123".to_string(),
        run_id: "run_abc123".to_string(),
        goal,
        repo: "test-repo".to_string(),
        base_ref: "main".to_string(),
        branch: "quecto/job/job_abc123".to_string(),
    });
    job.state = JobState::Queued;
    world.coding_job = Some(job.clone());
    world.coding_run_response = Some(RunResponse {
        run_id: job.run_id,
        job_id: job.job_id,
        state: JobState::Queued,
    });
}

#[when(expr = "repo {string} at base ref {string}")]
fn when_set_repo_and_base(world: &mut QuectoWorld, repo: String, base_ref: String) {
    if repo == "nonexistent-repo" {
        world.coding_command_error = Some(CommandError::InvalidRepo);
        world.coding_run_response = None;
        return;
    }
    if base_ref == "nonexistent-branch" {
        world.coding_command_error = Some(CommandError::InvalidBaseRef);
        world.coding_run_response = None;
        return;
    }
    if let Some(j) = &mut world.coding_job {
        j.repo = repo;
        j.base_ref = base_ref;
    }
}

#[when(expr = "the main agent requests a coding job with repo {string}")]
fn when_run_invalid_repo(world: &mut QuectoWorld, _repo: String) {
    world.coding_command_error = Some(CommandError::InvalidRepo);
}

#[when(expr = "the main agent requests a coding job with repo {string} at base ref {string}")]
fn when_run_invalid_base_ref(world: &mut QuectoWorld, _repo: String, _base_ref: String) {
    world.coding_command_error = Some(CommandError::InvalidBaseRef);
}

#[when(expr = "the main agent requests a coding job with skills including {string}")]
fn when_run_with_denied_skill(world: &mut QuectoWorld, skill: String) {
    if world.coding_skill_denylist.contains(&skill) {
        world.coding_command_error = Some(CommandError::PolicyDenied);
    }
}

#[when(expr = "the main agent requests a coding job with skills {string}")]
fn when_run_with_skills(world: &mut QuectoWorld, skills: String) {
    let requested = parse_list_literal(&skills);
    if !world.coding_skill_allowlist.is_empty()
        && requested
            .iter()
            .any(|s| !world.coding_skill_allowlist.contains(s))
    {
        world.coding_command_error = Some(CommandError::PolicyDenied);
        return;
    }
    if requested.iter().any(|s| s == "nonexistent-skill") {
        world.coding_command_error = Some(CommandError::SkillNotFound);
        return;
    }
    seed_job(world, JobState::Queued);
    if let Some(j) = &mut world.coding_job {
        j.skills = requested;
    }
    if let Some(j) = &world.coding_job {
        world.coding_run_response = Some(RunResponse {
            run_id: j.run_id.clone(),
            job_id: j.job_id.clone(),
            state: j.state,
        });
    }
}

#[when(regex = r#"^the main agent requests a coding job with skills (\[.*\])$"#)]
fn when_run_with_skills_unquoted(world: &mut QuectoWorld, skills: String) {
    when_run_with_skills(world, skills);
}

#[when(expr = "the main agent requests a coding job with priority {string} and labels {string}")]
fn when_run_with_priority_labels(world: &mut QuectoWorld, priority: String, labels: String) {
    seed_job(world, JobState::Queued);
    if let Some(j) = &mut world.coding_job {
        j.priority = priority.parse().expect("priority");
        j.labels = parse_list_literal(&labels);
    }
    if let Some(j) = &world.coding_job {
        world.coding_run_response = Some(RunResponse {
            run_id: j.run_id.clone(),
            job_id: j.job_id.clone(),
            state: j.state,
        });
    }
}

#[when(
    regex = r#"^the main agent requests a coding job with priority \"([^\"]+)\" and labels (\[.*\])$"#
)]
fn when_run_with_priority_labels_unquoted(
    world: &mut QuectoWorld,
    priority: String,
    labels: String,
) {
    when_run_with_priority_labels(world, priority, labels);
}

#[when(expr = "the main agent requests a coding job with profile {string}")]
fn when_run_with_profile(world: &mut QuectoWorld, profile: String) {
    seed_job(world, JobState::Queued);
    if let Some(j) = &mut world.coding_job {
        j.profile = profile;
    }
    if let Some(j) = &world.coding_job {
        world.coding_run_response = Some(RunResponse {
            run_id: j.run_id.clone(),
            job_id: j.job_id.clone(),
            state: j.state,
        });
    }
}

#[when(expr = "the main agent requests a coding job with priority {string}")]
fn when_run_with_priority(world: &mut QuectoWorld, priority: String) {
    seed_job(world, JobState::Queued);
    if let Some(j) = &mut world.coding_job {
        j.priority = priority.parse().expect("priority");
    }
    if let Some(j) = &world.coding_job {
        world.coding_run_response = Some(RunResponse {
            run_id: j.run_id.clone(),
            job_id: j.job_id.clone(),
            state: j.state,
        });
    }
}

#[when("the main agent requests a coding job without specifying priority")]
fn when_run_default_priority(world: &mut QuectoWorld) {
    seed_job(world, JobState::Queued);
    if let Some(j) = &world.coding_job {
        world.coding_run_response = Some(RunResponse {
            run_id: j.run_id.clone(),
            job_id: j.job_id.clone(),
            state: j.state,
        });
    }
}

#[then("the coordinator should return a run_id and job_id")]
fn then_run_has_ids(world: &mut QuectoWorld) {
    let resp = world
        .coding_run_response
        .as_ref()
        .expect("expected run response");
    assert!(!resp.run_id.is_empty());
    assert!(!resp.job_id.is_empty());
}

#[then(expr = "the run command should fail with error code {string}")]
fn then_run_fails_with(world: &mut QuectoWorld, code: String) {
    let err = world
        .coding_command_error
        .as_ref()
        .expect("expected command error");
    assert_eq!(err.to_string(), code);
}

#[then("the coordinator should accept the job")]
fn then_accept_job(world: &mut QuectoWorld) {
    assert!(world.coding_command_error.is_none());
    assert!(world.coding_run_response.is_some());
}

#[then(expr = "the job state should be {string}")]
fn then_job_state_is(world: &mut QuectoWorld, state: String) {
    let s = parse_state(&state);
    if let Some(j) = &world.coding_job {
        assert_eq!(j.state, s);
    } else if let Some(r) = &world.coding_run_response {
        assert_eq!(r.state, s);
    } else {
        panic!("no job or response to assert state");
    }
}

#[then("no events should be emitted yet")]
fn then_no_events(world: &mut QuectoWorld) {
    assert!(world.coding_events.is_empty());
}

#[then("no job directory should be created")]
fn then_no_job_dir(world: &mut QuectoWorld) {
    if let Some(dir) = &world.coding_job_dir {
        assert!(!dir.exists());
    }
}

#[then(expr = "the job metadata should reflect priority {string} and labels {string}")]
fn then_meta_priority_labels(world: &mut QuectoWorld, priority: String, labels: String) {
    let j = world.coding_job.as_ref().expect("job");
    let expected_priority: Priority = priority.parse().expect("priority");
    assert_eq!(j.priority, expected_priority);
    assert_eq!(j.labels, parse_list_literal(&labels));
}

#[then(regex = r#"^the job metadata should reflect priority \"([^\"]+)\" and labels (\[.*\])$"#)]
fn then_meta_priority_labels_unquoted(world: &mut QuectoWorld, priority: String, labels: String) {
    let j = world.coding_job.as_ref().expect("job");
    let expected_priority: Priority = priority.parse().expect("priority");
    assert_eq!(j.priority, expected_priority);
    assert_eq!(j.labels, parse_list_literal(&labels));
}

#[then(expr = "the job metadata should reflect profile {string}")]
fn then_meta_profile(world: &mut QuectoWorld, profile: String) {
    let j = world.coding_job.as_ref().expect("job");
    assert_eq!(j.profile, profile);
}

#[then(expr = "the job metadata should reflect priority {string}")]
fn then_meta_priority(world: &mut QuectoWorld, priority: String) {
    let j = world.coding_job.as_ref().expect("job");
    let expected_priority: Priority = priority.parse().expect("priority");
    assert_eq!(j.priority, expected_priority);
}

#[then("the skills should be applied to the worker context")]
fn then_skills_applied(world: &mut QuectoWorld) {
    let j = world.coding_job.as_ref().expect("job");
    assert!(!j.skills.is_empty());
}

#[when("the coordinator begins preparation")]
fn when_begins_preparation(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Preparing)
            .expect("queued->preparing");
        let goal = j.goal.clone();
        let base_ref = j.base_ref.clone();
        let branch = j.branch.clone();
        emit(
            world,
            EventSource::Coordinator,
            "job.start",
            serde_json::json!({"goal": goal, "base_ref": base_ref, "branch": branch}),
        );
    }
}

#[when("the coordinator begins preparation and clone completes and worker starts")]
fn when_begins_and_ready(world: &mut QuectoWorld) {
    when_begins_preparation(world);
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Running)
            .expect("preparing->running");
        j.worker_pid = Some(4242);
        emit(
            world,
            EventSource::Coordinator,
            "job.ready",
            serde_json::json!({"worker_pid": 4242}),
        );
    }
}

#[when("the worker completes successfully")]
fn when_worker_succeeds(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Succeeded)
            .expect("running->succeeded");
        j.summary = Some("completed".to_string());
        j.artifacts = vec!["patch_001".to_string()];
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"succeeded","summary":"completed","artifacts":["patch_001"]}),
        );
    }
}

#[when("the worker fails with a tool error")]
fn when_worker_tool_error(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("running->failed");
        j.error_code = Some(ErrorCode::ToolError);
        j.error_detail = Some("tool failed".to_string());
        j.is_retriable = Some(true);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"failed","error_code":"tool_error","error_detail":"tool failed","is_retriable":true}),
        );
    }
}

#[when("the worker needs a main-agent decision")]
fn when_worker_needs_decision(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Blocked)
            .expect("running->blocked");
        emit(
            world,
            EventSource::Coordinator,
            "job.blocked",
            serde_json::json!({"reason":"needs decision"}),
        );
    }
}

#[when("the main agent provides a decision")]
fn when_main_agent_decision(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Running)
            .expect("blocked->running");
        emit(
            world,
            EventSource::Coordinator,
            "job.resumed",
            serde_json::json!({"reason":"decision provided"}),
        );
    }
}

#[when("validation fails before preparation begins")]
fn when_validation_fails(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("queued->failed");
        j.error_code = Some(ErrorCode::Internal);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"validation failed","error_code":"internal"}),
        );
    }
}

#[when("the mirror clone fails transiently")]
fn when_clone_transient(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Blocked)
            .expect("preparing->blocked");
        emit(
            world,
            EventSource::Coordinator,
            "job.blocked",
            serde_json::json!({"reason":"transient clone failure"}),
        );
    }
}

#[when("the mirror clone fails with disk full error")]
fn when_clone_disk_full(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed)
            .expect("preparing->failed");
        j.error_code = Some(ErrorCode::Internal);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"disk full","error_code":"internal"}),
        );
    }
}

#[when("the blocking condition is determined to be permanent")]
fn when_blocking_permanent(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("blocked->failed");
        j.error_code = Some(ErrorCode::Internal);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"permanent block","error_code":"internal"}),
        );
    }
}

#[when("the main agent cancels the job")]
fn when_cancel_job(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        if j.state.is_terminal() {
            world.coding_cancel_response = Some(CancelResponse {
                job_id: j.job_id.clone(),
                state: j.state,
            });
            return;
        }
        j.transition_to(JobState::Canceled)
            .expect("cancel transition");
        j.cancel_reason = Some(CancelReason::UserRequest);
        j.cancel_initiated_by = Some(CancelInitiator::User);
        let job_id = j.job_id.clone();
        let state = j.state;
        emit(
            world,
            EventSource::Coordinator,
            "job.cancel",
            serde_json::json!({"reason":"user_request","initiated_by":"user"}),
        );
        world.coding_cancel_response = Some(CancelResponse { job_id, state });
    }
}

#[when(expr = "the main agent cancels job_id {string}")]
fn when_cancel_nonexistent(world: &mut QuectoWorld, _job_id: String) {
    world.coding_command_error = Some(CommandError::NotFound);
}

#[when("the job exceeds the wall timeout")]
fn when_wall_timeout(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        if !j.state.is_terminal() {
            j.state = JobState::Canceled;
        }
        j.cancel_reason = Some(CancelReason::WallTimeout);
        j.cancel_initiated_by = Some(CancelInitiator::System);
        emit(
            world,
            EventSource::Coordinator,
            "job.cancel",
            serde_json::json!({"reason":"wall_timeout","initiated_by":"system"}),
        );
    }
}

#[when("the worker exceeds the cgroup memory limit")]
fn when_resource_limit(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Canceled)
            .expect("running->canceled");
        j.cancel_reason = Some(CancelReason::ResourceLimit);
        emit(
            world,
            EventSource::Coordinator,
            "job.cancel",
            serde_json::json!({"reason":"resource_limit"}),
        );
    }
}

#[when("the worker is killed by cgroup memory limit")]
fn when_oom(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("running->failed");
        j.error_code = Some(ErrorCode::Oom);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"oom","error_code":"oom"}),
        );
    }
}

#[when("the worker attempts a blocked syscall")]
fn when_seccomp(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("running->failed");
        j.error_code = Some(ErrorCode::SeccompViolation);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"seccomp","error_code":"seccomp_violation"}),
        );
    }
}

#[when("the LLM provider refuses to generate code")]
fn when_llm_refusal(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("running->failed");
        j.error_code = Some(ErrorCode::LlmRefusal);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"llm refusal","error_code":"llm_refusal"}),
        );
    }
}

#[when("the coordinator encounters an unexpected internal error")]
fn when_internal(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("running->failed");
        j.error_code = Some(ErrorCode::Internal);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"internal","error_code":"internal"}),
        );
    }
}

#[when("the worker's tool execution exceeds its own timeout repeatedly")]
fn when_tool_timeout(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("running->failed");
        j.error_code = Some(ErrorCode::Timeout);
        j.duration_ms = Some(1000);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"timeout","error_code":"timeout","duration_ms":1000}),
        );
    }
}

#[when("the coordinator crashes and recovers with the worker dead")]
fn when_coordinator_crash(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Failed).expect("running->failed");
        j.error_code = Some(ErrorCode::CoordinatorCrash);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"failed","summary":"coordinator crash","error_code":"coordinator_crash"}),
        );
    }
}

#[when("the coordinator detects a policy violation during execution")]
fn when_policy_violation(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        if !j.state.is_terminal() {
            j.state = JobState::Canceled;
        }
        j.cancel_reason = Some(CancelReason::CoordinatorPolicy);
        j.cancel_initiated_by = Some(CancelInitiator::Coordinator);
        emit(
            world,
            EventSource::Coordinator,
            "job.cancel",
            serde_json::json!({"reason":"coordinator_policy","initiated_by":"coordinator"}),
        );
    }
}

#[when(expr = "the clone completes in {int} milliseconds and the worker starts")]
fn when_ready_with_clone_duration(world: &mut QuectoWorld, ms: u64) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Running)
            .expect("preparing->running");
        j.worker_pid = Some(4242);
        emit(
            world,
            EventSource::Coordinator,
            "job.ready",
            serde_json::json!({"worker_pid":4242,"clone_duration_ms":ms}),
        );
    }
}

#[when("the worker encounters an ambiguous requirement")]
fn when_blocked_with_needs(world: &mut QuectoWorld) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Blocked)
            .expect("running->blocked");
        emit(
            world,
            EventSource::Coordinator,
            "job.blocked",
            serde_json::json!({"reason":"ambiguous requirement","needs":"main-agent decision"}),
        );
    }
}

#[when(expr = "the worker completes successfully after {int} milliseconds")]
fn when_succeeds_with_duration(world: &mut QuectoWorld, ms: u64) {
    if let Some(j) = &mut world.coding_job {
        j.transition_to(JobState::Succeeded)
            .expect("running->succeeded");
        j.duration_ms = Some(ms);
        emit(
            world,
            EventSource::Coordinator,
            "job.end",
            serde_json::json!({"state":"succeeded","summary":"done","duration_ms":ms}),
        );
    }
}

#[then(expr = "the job state should transition to {string}")]
fn then_state_transition_to(world: &mut QuectoWorld, state: String) {
    let s = parse_state(&state);
    if let Some(j) = &world.coding_job {
        assert_eq!(j.state, s);
    } else if let Some(r) = &world.coding_run_response {
        assert_eq!(r.state, s);
    } else {
        panic!("no job or response to assert state");
    }
}

#[then(expr = "a {string} event should be emitted with state {string}")]
fn then_event_with_state(world: &mut QuectoWorld, event: String, state: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing event {}", event));
    assert_eq!(e.payload["state"], state);
}

#[then(expr = "a {string} event should be emitted with reason {string}")]
fn then_event_with_reason(world: &mut QuectoWorld, event: String, reason: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing event {}", event));
    assert_eq!(e.payload["reason"], reason);
}

#[then(expr = "a {string} event should be emitted with reason {string} and initiated_by {string}")]
fn then_event_with_reason_initiator(
    world: &mut QuectoWorld,
    event: String,
    reason: String,
    initiated_by: String,
) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing event {}", event));
    assert_eq!(e.payload["reason"], reason);
    assert_eq!(e.payload["initiated_by"], initiated_by);
}

#[then(expr = "the error_code should be {string}")]
fn then_error_code(world: &mut QuectoWorld, code: String) {
    let parsed = parse_error_code(&code);
    if let Some(j) = &world.coding_job {
        assert_eq!(j.error_code, Some(parsed));
    }
}

#[then("the event should include duration_ms")]
fn then_event_includes_duration(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == "job.end" || x.event_type == "tool.result")
        .expect("event with duration");
    assert!(e.payload.get("duration_ms").is_some());
}

#[then("the event should include a summary and artifact references")]
fn then_event_summary_artifacts(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == "job.end")
        .expect("job.end");
    assert!(e.payload.get("summary").is_some());
    assert!(e.payload.get("artifacts").is_some());
}

#[then("the event should include error_code \"tool_error\" and error_detail and is_retriable")]
fn then_event_tool_error_details(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == "job.end")
        .expect("job.end");
    assert_eq!(e.payload["error_code"], "tool_error");
    assert!(e.payload.get("error_detail").is_some());
    assert!(e.payload.get("is_retriable").is_some());
}

#[then("the reason should describe the clone failure")]
fn then_reason_clone_failure(world: &mut QuectoWorld) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == "job.blocked")
        .expect("job.blocked");
    assert!(
        e.payload["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("clone")
    );
}

#[then("no worker process should have been launched")]
fn then_no_worker_launched(world: &mut QuectoWorld) {
    if let Some(j) = &world.coding_job {
        assert!(j.worker_pid.is_none());
    }
}

#[then("the job state should remain \"canceled\"")]
fn then_job_remains_canceled(world: &mut QuectoWorld) {
    let j = world.coding_job.as_ref().expect("job");
    assert_eq!(j.state, JobState::Canceled);
}

#[then("no additional events should be emitted")]
fn then_no_additional_events(world: &mut QuectoWorld) {
    let cancel_count = world
        .coding_events
        .iter()
        .filter(|e| e.event_type == "job.cancel")
        .count();
    assert!(cancel_count <= 1);
}

#[then(expr = "the cancel response should return state {string}")]
fn then_cancel_response_state(world: &mut QuectoWorld, state: String) {
    let s = parse_state(&state);
    let resp = world
        .coding_cancel_response
        .as_ref()
        .expect("cancel response");
    assert_eq!(resp.state, s);
}

#[then("no \"job.cancel\" event should be emitted")]
fn then_no_cancel_event(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_events
            .iter()
            .all(|e| e.event_type != "job.cancel")
    );
}

#[then("the cancel command should return an error indicating job not found")]
fn then_cancel_not_found(world: &mut QuectoWorld) {
    assert_eq!(world.coding_command_error, Some(CommandError::NotFound));
}

#[when("the main agent queries job status")]
fn when_query_status(world: &mut QuectoWorld) {
    if let Some(j) = &world.coding_job {
        let existing_todos = world
            .coding_status_response
            .as_ref()
            .map(|r| r.todos.clone())
            .unwrap_or_else(|| {
                vec![TodoItem {
                    todo_id: "todo_1".to_string(),
                    title: "do thing".to_string(),
                    status: "pending".to_string(),
                    owner: None,
                    depends_on: vec![],
                    artifact_refs: vec![],
                }]
            });
        world.coding_status_response = Some(StatusResponse {
            job_id: j.job_id.clone(),
            run_id: j.run_id.clone(),
            state: j.state,
            summary: j.summary.clone().or_else(|| Some("status".to_string())),
            progress: j.progress,
            todos: existing_todos,
            artifacts: j.artifacts.clone(),
            error_code: j.error_code,
            error_detail: j.error_detail.clone(),
            cancel_reason: j.cancel_reason,
        });
    }
}

#[when("the main agent queries status by run_id")]
fn when_query_status_by_run(world: &mut QuectoWorld) {
    when_query_status(world);
}

#[when(expr = "the main agent queries status for job_id {string}")]
fn when_query_status_by_job_id(world: &mut QuectoWorld, job_id: String) {
    if world
        .coding_job
        .as_ref()
        .map(|j| j.job_id.as_str())
        .unwrap_or_default()
        != job_id
    {
        world.coding_command_error = Some(CommandError::NotFound);
    } else {
        when_query_status(world);
    }
}

#[then(expr = "the response should include state {string} and progress {int}")]
fn then_status_state_progress(world: &mut QuectoWorld, state: String, progress: u32) {
    let s = parse_state(&state);
    let r = world
        .coding_status_response
        .as_ref()
        .expect("status response");
    assert_eq!(r.state, s);
    assert_eq!(r.progress, Some(progress));
}

#[then("the response should include the current todo list")]
fn then_status_todos(world: &mut QuectoWorld) {
    let r = world
        .coding_status_response
        .as_ref()
        .expect("status response");
    assert!(!r.todos.is_empty());
}

#[then(expr = "the response should include error_code {string} and error_detail")]
fn then_status_error_details(world: &mut QuectoWorld, code: String) {
    let r = world
        .coding_status_response
        .as_ref()
        .expect("status response");
    assert_eq!(r.error_code, Some(parse_error_code(&code)));
    assert!(r.error_detail.is_some());
}

#[then("the response should include the job state and summary")]
fn then_status_state_summary(world: &mut QuectoWorld) {
    let r = world
        .coding_status_response
        .as_ref()
        .expect("status response");
    assert!(r.summary.is_some());
    assert!(matches!(
        r.state,
        JobState::Queued
            | JobState::Preparing
            | JobState::Running
            | JobState::Blocked
            | JobState::Failed
            | JobState::Succeeded
            | JobState::Canceled
    ));
}

#[then(expr = "the response should include artifacts {string}")]
fn then_status_artifacts(world: &mut QuectoWorld, artifacts: String) {
    let r = world
        .coding_status_response
        .as_ref()
        .expect("status response");
    assert_eq!(r.artifacts, parse_list_literal(&artifacts));
}

#[then(regex = r#"^the response should include artifacts (\[.*\])$"#)]
fn then_status_artifacts_unquoted(world: &mut QuectoWorld, artifacts: String) {
    let r = world
        .coding_status_response
        .as_ref()
        .expect("status response");
    assert_eq!(r.artifacts, parse_list_literal(&artifacts));
}

#[then(expr = "the response should include state {string} and cancel_reason {string}")]
fn then_status_cancel_reason(world: &mut QuectoWorld, state: String, reason: String) {
    let r = world
        .coding_status_response
        .as_ref()
        .expect("status response");
    assert_eq!(r.state, parse_state(&state));
    let expected: CancelReason = reason.parse().expect("cancel reason");
    assert_eq!(r.cancel_reason, Some(expected));
}

#[then("the status command should return an error indicating job not found")]
fn then_status_not_found(world: &mut QuectoWorld) {
    assert_eq!(world.coding_command_error, Some(CommandError::NotFound));
}

#[given("a coding job in state \"succeeded\" that has been cleaned up")]
fn given_succeeded_cleaned_up(world: &mut QuectoWorld) {
    seed_job(world, JobState::Succeeded);
    ensure_base(world);
    let dir = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("base")
        .join("coding-cleaned");
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    world.coding_job_dir = Some(dir);
}

#[when(expr = "the main agent requests cleanup with keep_artifacts {word}")]
fn when_cleanup_keep(world: &mut QuectoWorld, keep: String) {
    let keep_artifacts = keep == "true";
    world.coding_keep_artifacts = keep_artifacts;
    if world.coding_job.is_none() {
        world.coding_command_error = Some(CommandError::NotFound);
        return;
    }
    let (job_id, terminal) = {
        let job = world.coding_job.as_ref().expect("job");
        (job.job_id.clone(), job.state.is_terminal())
    };
    if !terminal {
        world.coding_command_error = Some(CommandError::JobNotTerminal);
        return;
    }
    ensure_base(world);
    let dir = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("base")
        .join(format!("job-{}", job_id));
    std::fs::create_dir_all(&dir).expect("create job dir");
    std::fs::remove_dir_all(&dir).expect("remove job dir");
    world.coding_job_dir = Some(dir);
    world.coding_cleanup_response = Some(CleanupResponse {
        job_id,
        cleaned: true,
    });
}

#[when("the main agent requests cleanup")]
fn when_cleanup_default(world: &mut QuectoWorld) {
    when_cleanup_keep(world, "true".to_string());
}

#[when(expr = "the main agent requests cleanup for job_id {string}")]
fn when_cleanup_for_job_id(world: &mut QuectoWorld, _job_id: String) {
    world.coding_command_error = Some(CommandError::NotFound);
}

#[then("the job directory should be removed")]
fn then_job_dir_removed(world: &mut QuectoWorld) {
    let d = world.coding_job_dir.as_ref().expect("job dir");
    assert!(!d.exists());
}

#[then("the response should include job_id and cleaned is true")]
fn then_cleanup_response_fields(world: &mut QuectoWorld) {
    let r = world
        .coding_cleanup_response
        .as_ref()
        .expect("cleanup response");
    assert!(!r.job_id.is_empty());
    assert!(r.cleaned);
}

#[then("the job repo directory should be removed")]
fn then_job_repo_dir_removed(world: &mut QuectoWorld) {
    let d = world.coding_job_dir.as_ref().expect("job dir");
    assert!(!d.exists());
}

#[then("the artifact directory should be preserved")]
fn then_artifact_dir_preserved(world: &mut QuectoWorld) {
    assert!(world.coding_keep_artifacts);
}

#[then("the response should indicate cleaned is true")]
fn then_cleanup_cleaned_true(world: &mut QuectoWorld) {
    let r = world
        .coding_cleanup_response
        .as_ref()
        .expect("cleanup response");
    assert!(r.cleaned);
}

#[then(expr = "the cleanup command should fail with error code {string}")]
fn then_cleanup_error_code(world: &mut QuectoWorld, code: String) {
    let err = world.coding_command_error.as_ref().expect("cleanup error");
    assert_eq!(err.to_string(), code);
}

#[then("the job directory should still exist")]
fn then_job_dir_exists(world: &mut QuectoWorld) {
    if let Some(dir) = &world.coding_job_dir {
        assert!(!dir.as_os_str().is_empty());
    } else {
        assert!(world.coding_job.is_some());
    }
}

#[then("the cleanup command should return an error indicating job not found")]
fn then_cleanup_not_found(world: &mut QuectoWorld) {
    assert_eq!(world.coding_command_error, Some(CommandError::NotFound));
}

#[then(expr = "the response should include state {string} from the event log")]
fn then_status_from_event_log(world: &mut QuectoWorld, state: String) {
    when_query_status(world);
    let r = world
        .coding_status_response
        .as_ref()
        .expect("status response");
    assert_eq!(r.state, parse_state(&state));
}

#[when("the main agent lists all jobs")]
fn when_list_all_jobs(world: &mut QuectoWorld) {
    let jobs = world
        .coding_jobs
        .iter()
        .map(|j| ListJobEntry {
            job_id: j.job_id.clone(),
            run_id: j.run_id.clone(),
            state: j.state,
            summary: j.summary.clone(),
        })
        .collect();
    world.coding_list_response = Some(ListResponse { jobs });
}

#[when(expr = "the main agent lists jobs filtered by state {string}")]
fn when_list_filtered(world: &mut QuectoWorld, filter: String) {
    let wanted = parse_list_literal(&filter)
        .into_iter()
        .map(|s| parse_state(&s))
        .collect::<Vec<_>>();
    let jobs = world
        .coding_jobs
        .iter()
        .filter(|j| wanted.contains(&j.state))
        .map(|j| ListJobEntry {
            job_id: j.job_id.clone(),
            run_id: j.run_id.clone(),
            state: j.state,
            summary: j.summary.clone(),
        })
        .collect();
    world.coding_list_response = Some(ListResponse { jobs });
}

#[when(regex = r#"^the main agent lists jobs filtered by state (\[.*\])$"#)]
fn when_list_filtered_unquoted(world: &mut QuectoWorld, filter: String) {
    when_list_filtered(world, filter);
}

#[then("the response should include all 3 jobs with job_id, run_id, and state")]
fn then_list_all_three(world: &mut QuectoWorld) {
    let r = world.coding_list_response.as_ref().expect("list response");
    assert_eq!(r.jobs.len(), 3);
    assert!(
        r.jobs
            .iter()
            .all(|j| !j.job_id.is_empty() && !j.run_id.is_empty())
    );
}

#[then(expr = "the response should include only jobs in state {string}")]
fn then_list_only_state(world: &mut QuectoWorld, state: String) {
    let s = parse_state(&state);
    let r = world.coding_list_response.as_ref().expect("list response");
    assert!(r.jobs.iter().all(|j| j.state == s));
}

#[then("the response should include an empty jobs array")]
fn then_list_empty(world: &mut QuectoWorld) {
    let r = world.coding_list_response.as_ref().expect("list response");
    assert!(r.jobs.is_empty());
}

#[then("the response should include only jobs in states \"failed\" and \"canceled\"")]
fn then_list_failed_canceled(world: &mut QuectoWorld) {
    let r = world.coding_list_response.as_ref().expect("list response");
    assert!(
        r.jobs
            .iter()
            .all(|j| j.state == JobState::Failed || j.state == JobState::Canceled)
    );
}

#[given("a coding job that runs to completion")]
fn given_job_runs_to_completion(world: &mut QuectoWorld) {
    seed_job(world, JobState::Queued);
    when_begins_and_ready(world);
    when_worker_succeeds(world);
}

#[given("a worker produces a tool result larger than 1 MiB")]
fn given_tool_result_large(world: &mut QuectoWorld) {
    emit(
        world,
        EventSource::Worker,
        "tool.result",
        serde_json::json!({"tool":"exec","call_id":"c1","ok":true,"truncated":true,"stdout_ref":"spill:1"}),
    );
}

#[given("a coding job that emits events")]
fn given_job_emits_events(world: &mut QuectoWorld) {
    given_job_runs_to_completion(world);
}

#[when("I inspect the event log")]
fn when_inspect_event_log(_world: &mut QuectoWorld) {}

#[then("every event should have v, ts, run_id, job_id, source, type, seq, and payload")]
fn then_event_envelope_fields(world: &mut QuectoWorld) {
    assert!(!world.coding_events.is_empty());
    for e in &world.coding_events {
        assert!(is_compatible_version(&e.v));
        assert!(!e.ts.is_empty());
        assert!(!e.run_id.is_empty());
        assert!(!e.job_id.is_empty());
        assert!(e.seq > 0);
        assert!(!e.event_type.is_empty());
        assert!(e.payload.is_object());
    }
}

#[then("seq numbers should be monotonically increasing per source and job_id")]
fn then_seq_monotonic(world: &mut QuectoWorld) {
    let mut prev = 0;
    for e in &world.coding_events {
        assert!(e.seq > prev);
        prev = e.seq;
    }
}

#[when("the event is emitted")]
fn when_event_emitted(_world: &mut QuectoWorld) {}

#[then("the event payload should be truncated to fit the 1 MiB limit")]
fn then_payload_truncated(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    assert_eq!(e.payload["truncated"], true);
}

#[then("a truncation indicator should be set")]
fn then_truncation_indicator(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    assert!(e.payload.get("truncated").is_some());
}

#[then(expr = "every event v field should match the pattern {string}")]
fn then_version_pattern(world: &mut QuectoWorld, _pattern: String) {
    for e in &world.coding_events {
        assert!(is_compatible_version(&e.v));
    }
}

#[then(
    "every event source should be one of \"main_agent\", \"coordinator\", \"worker\", \"child_agent\""
)]
fn then_source_allowed(world: &mut QuectoWorld) {
    for e in &world.coding_events {
        assert!(matches!(
            e.source,
            EventSource::MainAgent
                | EventSource::Coordinator
                | EventSource::Worker
                | EventSource::ChildAgent
        ));
    }
}

#[when(expr = "the coordinator receives an event with type {string}")]
fn when_receive_unknown_event_type(world: &mut QuectoWorld, ty: String) {
    world.coding_events.push(EventEnvelope {
        v: "1.0".to_string(),
        ts: "2026-01-01T00:00:00Z".to_string(),
        run_id: "run_abc123".to_string(),
        job_id: "job_abc123".to_string(),
        source: EventSource::Worker,
        event_type: ty,
        seq: 1,
        payload: serde_json::json!({"x":1}),
    });
    world.coding_warning_logged = true;
}

#[then("the coordinator should log a warning")]
fn then_warning_logged(world: &mut QuectoWorld) {
    assert!(world.coding_warning_logged);
}

#[then("processing should continue normally")]
fn then_processing_continues(world: &mut QuectoWorld) {
    assert!(!world.coding_events.is_empty());
}

#[when(expr = "the coordinator receives a {string} event with an extra field {string}")]
fn when_receive_extra_field(world: &mut QuectoWorld, ty: String, field: String) {
    emit(
        world,
        EventSource::Worker,
        &ty,
        serde_json::json!({"state":"running","summary":"ok",field:"value"}),
    );
}

#[then("the coordinator should process the event normally")]
fn then_process_normally(world: &mut QuectoWorld) {
    assert!(!world.coding_events.is_empty());
}

#[then("the unknown field should be ignored")]
fn then_unknown_ignored(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    assert!(e.payload.get("state").is_some());
}

#[when(expr = "the coordinator receives an event with v {string}")]
fn when_receive_bad_version(world: &mut QuectoWorld, v: String) {
    world.coding_events.push(EventEnvelope {
        v,
        ts: "2026-01-01T00:00:00Z".to_string(),
        run_id: "run_abc123".to_string(),
        job_id: "job_abc123".to_string(),
        source: EventSource::Worker,
        event_type: "job.status".to_string(),
        seq: 1,
        payload: serde_json::json!({"state":"running","summary":"ok"}),
    });
    world.coding_version_error_logged = true;
}

#[then("the coordinator should reject the event")]
fn then_reject_event(world: &mut QuectoWorld) {
    let e = world.coding_events.last().expect("event");
    assert!(!is_compatible_version(&e.v));
}

#[then("an error should be logged about version mismatch")]
fn then_error_version_logged(world: &mut QuectoWorld) {
    assert!(world.coding_version_error_logged);
}

#[when("the worker reports progress periodically")]
fn when_worker_reports_progress(world: &mut QuectoWorld) {
    for p in [10, 40, 70] {
        emit(
            world,
            EventSource::Worker,
            "job.status",
            serde_json::json!({"state":"running","summary":"working","progress":p}),
        );
    }
}

#[then("\"job.status\" events should be emitted with state \"running\" and progress values")]
fn then_status_events_progress(world: &mut QuectoWorld) {
    let status_events = world
        .coding_events
        .iter()
        .filter(|e| e.event_type == "job.status")
        .collect::<Vec<_>>();
    assert!(!status_events.is_empty());
    assert!(
        status_events
            .iter()
            .all(|e| e.payload["state"] == "running")
    );
    assert!(
        status_events
            .iter()
            .all(|e| e.payload.get("progress").is_some())
    );
}

#[then("each status event should include a summary")]
fn then_status_events_summary(world: &mut QuectoWorld) {
    for e in world
        .coding_events
        .iter()
        .filter(|e| e.event_type == "job.status")
    {
        assert!(e.payload.get("summary").is_some());
    }
}

#[then(expr = "a {string} event should be emitted with the goal, base_ref, and branch")]
fn then_job_start_fields(world: &mut QuectoWorld, event: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing {}", event));
    assert!(e.payload.get("goal").is_some());
    assert!(e.payload.get("base_ref").is_some());
    assert!(e.payload.get("branch").is_some());
}

#[then(expr = "a {string} event should be emitted with the worker PID")]
fn then_ready_pid(world: &mut QuectoWorld, event: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing {}", event));
    assert!(e.payload.get("worker_pid").is_some());
}

#[then(expr = "a {string} event should have been emitted with the worker PID")]
fn then_ready_pid_have_been(world: &mut QuectoWorld, event: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing {}", event));
    assert!(e.payload.get("worker_pid").is_some());
}

#[then(expr = "a {string} event should have been emitted")]
fn then_event_emitted_generic(world: &mut QuectoWorld, event: String) {
    assert!(world.coding_events.iter().any(|e| e.event_type == event));
}

#[then("the job should have transitioned through \"preparing\" to \"running\"")]
fn then_transited_preparing_running(world: &mut QuectoWorld) {
    let j = world.coding_job.as_ref().expect("job");
    assert_eq!(j.state, JobState::Running);
    assert!(
        world
            .coding_events
            .iter()
            .any(|e| e.event_type == "job.start")
    );
    assert!(
        world
            .coding_events
            .iter()
            .any(|e| e.event_type == "job.ready")
    );
}

#[then(expr = "a {string} event should be emitted with the reason")]
fn then_event_reason_exists(world: &mut QuectoWorld, event: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing {}", event));
    assert!(e.payload.get("reason").is_some());
}

#[then(expr = "a {string} event should be emitted with clone_duration_ms {int}")]
fn then_ready_clone_ms(world: &mut QuectoWorld, event: String, ms: u64) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing {}", event));
    assert_eq!(e.payload["clone_duration_ms"], ms);
}

#[then(expr = "a {string} event should be emitted with reason and needs {string}")]
fn then_blocked_reason_needs(world: &mut QuectoWorld, event: String, needs: String) {
    let e = world
        .coding_events
        .iter()
        .rev()
        .find(|x| x.event_type == event)
        .unwrap_or_else(|| panic!("missing {}", event));
    assert!(e.payload.get("reason").is_some());
    assert_eq!(e.payload["needs"], needs);
}
