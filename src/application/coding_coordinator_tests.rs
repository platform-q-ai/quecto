use super::*;
use crate::domain::coding_job::Priority;

struct MockRepoValidator {
    valid_repos: Vec<String>,
    valid_refs: Vec<(String, String)>,
}

impl RepoValidator for MockRepoValidator {
    fn repo_exists(&self, repo: &str) -> bool {
        self.valid_repos.iter().any(|r| r == repo)
    }
    fn ref_exists(&self, repo: &str, base_ref: &str) -> bool {
        self.valid_refs
            .iter()
            .any(|(r, b)| r == repo && b == base_ref)
    }
}

struct MockSkillResolver {
    available: Vec<String>,
}

impl SkillResolver for MockSkillResolver {
    fn skill_exists(&self, name: &str) -> bool {
        self.available.iter().any(|s| s == name)
    }
}

fn test_coordinator() -> CodingCoordinator<MockRepoValidator, MockSkillResolver> {
    CodingCoordinator::new(
        MockRepoValidator {
            valid_repos: vec!["test-repo".to_string()],
            valid_refs: vec![("test-repo".to_string(), "main".to_string())],
        },
        MockSkillResolver {
            available: vec!["rust-style".to_string(), "test-first".to_string()],
        },
        CoordinatorPolicy::default(),
    )
}

fn run_default(coord: &mut CodingCoordinator<MockRepoValidator, MockSkillResolver>) -> RunResponse {
    coord
        .run(RunRequest {
            goal: "test goal".to_string(),
            repo: "test-repo".to_string(),
            base_ref: "main".to_string(),
            priority: Priority::default(),
            profile: "default".to_string(),
            max_wall_seconds: None,
            labels: vec![],
            skills: vec![],
        })
        .unwrap()
}

#[test]
fn test_run_creates_queued_job() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    assert!(!resp.run_id.is_empty());
    assert!(!resp.job_id.is_empty());
    assert_eq!(resp.state, JobState::Queued);
    assert!(coord.events().is_empty());
}

#[test]
fn test_run_rejects_invalid_repo() {
    let mut coord = test_coordinator();
    let err = coord
        .run(RunRequest {
            goal: "g".to_string(),
            repo: "nonexistent-repo".to_string(),
            base_ref: "main".to_string(),
            priority: Priority::default(),
            profile: "default".to_string(),
            max_wall_seconds: None,
            labels: vec![],
            skills: vec![],
        })
        .unwrap_err();
    assert_eq!(err, CommandError::InvalidRepo);
}

#[test]
fn test_run_rejects_invalid_base_ref() {
    let mut coord = test_coordinator();
    let err = coord
        .run(RunRequest {
            goal: "g".to_string(),
            repo: "test-repo".to_string(),
            base_ref: "nonexistent-branch".to_string(),
            priority: Priority::default(),
            profile: "default".to_string(),
            max_wall_seconds: None,
            labels: vec![],
            skills: vec![],
        })
        .unwrap_err();
    // Now returns the enriched variant so callers get the default branch and
    // available refs inline — the display string starts with "invalid_base_ref: ".
    assert!(
        matches!(err, CommandError::InvalidBaseRefDetail(_)),
        "expected InvalidBaseRefDetail, got {err:?}"
    );
    let detail = err.to_string();
    assert!(
        detail.starts_with("invalid_base_ref: "),
        "detail should start with 'invalid_base_ref: ', got: {detail}"
    );
    assert!(
        detail.contains("default_branch="),
        "detail should contain default_branch hint, got: {detail}"
    );
}

#[test]
fn test_run_rejects_denied_skill() {
    let mut coord = CodingCoordinator::new(
        MockRepoValidator {
            valid_repos: vec!["test-repo".to_string()],
            valid_refs: vec![("test-repo".to_string(), "main".to_string())],
        },
        MockSkillResolver { available: vec![] },
        CoordinatorPolicy {
            skill_denylist: vec!["forbidden-skill".to_string()],
            skill_allowlist: vec![],
            max_retained_jobs: None,
        },
    );
    let err = coord
        .run(RunRequest {
            goal: "g".to_string(),
            repo: "test-repo".to_string(),
            base_ref: "main".to_string(),
            priority: Priority::default(),
            profile: "default".to_string(),
            max_wall_seconds: None,
            labels: vec![],
            skills: vec!["forbidden-skill".to_string()],
        })
        .unwrap_err();
    assert_eq!(err, CommandError::PolicyDenied);
}

#[test]
fn test_run_rejects_missing_skill() {
    let mut coord = CodingCoordinator::new(
        MockRepoValidator {
            valid_repos: vec!["test-repo".to_string()],
            valid_refs: vec![("test-repo".to_string(), "main".to_string())],
        },
        MockSkillResolver { available: vec![] },
        CoordinatorPolicy {
            skill_denylist: vec![],
            skill_allowlist: vec!["nonexistent-skill".to_string()],
            max_retained_jobs: None,
        },
    );
    let err = coord
        .run(RunRequest {
            goal: "g".to_string(),
            repo: "test-repo".to_string(),
            base_ref: "main".to_string(),
            priority: Priority::default(),
            profile: "default".to_string(),
            max_wall_seconds: None,
            labels: vec![],
            skills: vec!["nonexistent-skill".to_string()],
        })
        .unwrap_err();
    assert_eq!(err, CommandError::SkillNotFound);
}

#[test]
fn test_run_rejects_when_max_retained_jobs_reached() {
    let mut coord = CodingCoordinator::new(
        MockRepoValidator {
            valid_repos: vec!["test-repo".to_string()],
            valid_refs: vec![("test-repo".to_string(), "main".to_string())],
        },
        MockSkillResolver {
            available: vec!["rust-style".to_string()],
        },
        CoordinatorPolicy {
            skill_denylist: vec![],
            skill_allowlist: vec![],
            max_retained_jobs: Some(1),
        },
    );

    let first = coord.run(RunRequest {
        goal: "g1".to_string(),
        repo: "test-repo".to_string(),
        base_ref: "main".to_string(),
        priority: Priority::default(),
        profile: "default".to_string(),
        max_wall_seconds: None,
        labels: vec![],
        skills: vec![],
    });
    assert!(first.is_ok());

    let second = coord.run(RunRequest {
        goal: "g2".to_string(),
        repo: "test-repo".to_string(),
        base_ref: "main".to_string(),
        priority: Priority::default(),
        profile: "default".to_string(),
        max_wall_seconds: None,
        labels: vec![],
        skills: vec![],
    });
    assert_eq!(second.unwrap_err(), CommandError::PolicyDenied);
}

#[test]
fn test_begin_preparation_emits_job_start() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    let job = coord.job(&resp.job_id).unwrap();
    assert_eq!(job.state, JobState::Preparing);
    assert_eq!(coord.events().len(), 1);
    assert_eq!(coord.events()[0].event_type, "job.start");
    assert!(coord.events()[0].payload.get("goal").is_some());
}

#[test]
fn test_full_lifecycle_queued_to_succeeded() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 4242, None).unwrap();
    coord
        .mark_succeeded(SuccessInfo {
            job_id: &resp.job_id,
            summary: "done",
            artifacts: vec!["patch_001".to_string()],
            duration_ms: None,
        })
        .unwrap();
    let job = coord.job(&resp.job_id).unwrap();
    assert_eq!(job.state, JobState::Succeeded);
    assert_eq!(coord.events().len(), 3);
}

#[test]
fn test_cancel_running_job() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    let cancel = coord.cancel(&resp.job_id).unwrap();
    assert_eq!(cancel.state, JobState::Canceled);
    let last = coord.events().last().unwrap();
    assert_eq!(last.event_type, "job.cancel");
    assert_eq!(last.payload["reason"], "user_request");
}

#[test]
fn test_cancel_succeeded_is_noop() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .mark_succeeded(SuccessInfo {
            job_id: &resp.job_id,
            summary: "ok",
            artifacts: vec![],
            duration_ms: None,
        })
        .unwrap();
    let event_count = coord.events().len();
    let cancel = coord.cancel(&resp.job_id).unwrap();
    assert_eq!(cancel.state, JobState::Succeeded);
    assert_eq!(coord.events().len(), event_count);
}

#[test]
fn test_cancel_nonexistent() {
    let mut coord = test_coordinator();
    let err = coord.cancel("nonexistent").unwrap_err();
    assert_eq!(err, CommandError::NotFound);
}

#[test]
fn test_cleanup_running_rejected() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    let err = coord.cleanup(&resp.job_id, false).unwrap_err();
    assert_eq!(err, CommandError::JobNotTerminal);
}

#[test]
fn test_cleanup_succeeded() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .mark_succeeded(SuccessInfo {
            job_id: &resp.job_id,
            summary: "ok",
            artifacts: vec![],
            duration_ms: None,
        })
        .unwrap();
    let cleanup = coord.cleanup(&resp.job_id, false).unwrap();
    assert!(cleanup.cleaned);
    // cleanup should remove the job from the coordinator
    assert!(coord.job(&resp.job_id).is_none());
}

#[test]
fn test_cleanup_removes_from_run_index() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .mark_succeeded(SuccessInfo {
            job_id: &resp.job_id,
            summary: "ok",
            artifacts: vec![],
            duration_ms: None,
        })
        .unwrap();
    coord.cleanup(&resp.job_id, false).unwrap();
    assert!(coord.status_by_run_id(&resp.run_id).is_err());
}

#[test]
fn test_list_all() {
    let mut coord = test_coordinator();
    run_default(&mut coord);
    run_default(&mut coord);
    let list = coord.list(&ListRequest { state_filter: None });
    assert_eq!(list.jobs.len(), 2);
}

#[test]
fn test_list_filtered() {
    let mut coord = test_coordinator();
    let r1 = run_default(&mut coord);
    run_default(&mut coord);
    coord.begin_preparation(&r1.job_id).unwrap();
    coord.mark_ready(&r1.job_id, 42, None).unwrap();
    let list = coord.list(&ListRequest {
        state_filter: Some(vec![JobState::Running]),
    });
    assert_eq!(list.jobs.len(), 1);
    assert_eq!(list.jobs[0].state, JobState::Running);
}

#[test]
fn test_status_by_job_id() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    let status = coord.status_by_job_id(&resp.job_id).unwrap();
    assert_eq!(status.state, JobState::Queued);
}

#[test]
fn test_status_by_run_id() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    let status = coord.status_by_run_id(&resp.run_id).unwrap();
    assert_eq!(status.state, JobState::Queued);
}

#[test]
fn test_status_nonexistent() {
    let coord = test_coordinator();
    let err = coord.status_by_job_id("nonexistent").unwrap_err();
    assert_eq!(err, CommandError::NotFound);
}

#[test]
fn test_status_returns_empty_todos() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    let status = coord.status_by_job_id(&resp.job_id).unwrap();
    assert!(status.todos.is_empty());
}

#[test]
fn test_mark_blocked_and_resume() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .mark_blocked(&resp.job_id, "needs decision", None)
        .unwrap();
    assert_eq!(coord.job(&resp.job_id).unwrap().state, JobState::Blocked);
    coord
        .mark_resumed(&resp.job_id, "decision provided")
        .unwrap();
    assert_eq!(coord.job(&resp.job_id).unwrap().state, JobState::Running);
}

#[test]
fn test_worker_progress() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .record_worker_progress(&resp.job_id, 50, "halfway")
        .unwrap();
    let job = coord.job(&resp.job_id).unwrap();
    assert_eq!(job.progress, Some(50));
}

#[test]
fn test_worker_progress_rejects_terminal_state() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .mark_succeeded(SuccessInfo {
            job_id: &resp.job_id,
            summary: "done",
            artifacts: vec![],
            duration_ms: None,
        })
        .unwrap();
    let err = coord
        .record_worker_progress(&resp.job_id, 50, "late update")
        .unwrap_err();
    assert_eq!(err, CommandError::InvalidTransition);
}

#[test]
fn test_emit_worker_event_rejects_terminal_state() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .mark_succeeded(SuccessInfo {
            job_id: &resp.job_id,
            summary: "done",
            artifacts: vec![],
            duration_ms: None,
        })
        .unwrap();
    let err = coord
        .emit_worker_event(&resp.job_id, "tool.result", serde_json::json!({"ok": true}))
        .unwrap_err();
    assert_eq!(err, CommandError::InvalidTransition);
}

#[test]
fn test_run_with_priority_and_labels() {
    let mut coord = test_coordinator();
    let resp = coord
        .run(RunRequest {
            goal: "g".to_string(),
            repo: "test-repo".to_string(),
            base_ref: "main".to_string(),
            priority: Priority::High,
            profile: "default".to_string(),
            max_wall_seconds: None,
            labels: vec!["urgent".to_string(), "bugfix".to_string()],
            skills: vec![],
        })
        .unwrap();
    let job = coord.job(&resp.job_id).unwrap();
    assert_eq!(job.priority, Priority::High);
    assert_eq!(job.labels, vec!["urgent".to_string(), "bugfix".to_string()]);
}

#[test]
fn test_wall_timeout_cancel() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    let cancel = coord
        .cancel_with_reason(
            &resp.job_id,
            CancelReason::WallTimeout,
            CancelInitiator::System,
        )
        .unwrap();
    assert_eq!(cancel.state, JobState::Canceled);
    let last = coord.events().last().unwrap();
    assert_eq!(last.payload["reason"], "wall_timeout");
    assert_eq!(last.payload["initiated_by"], "system");
}

#[test]
fn test_mark_ready_with_clone_duration() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, Some(1200)).unwrap();
    let ready_event = coord
        .events()
        .iter()
        .find(|e| e.event_type == "job.ready")
        .unwrap();
    assert_eq!(ready_event.payload["clone_duration_ms"], 1200);
}

#[test]
fn test_mark_failed_with_details() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .mark_failed(FailureInfo {
            job_id: &resp.job_id,
            error_code: ErrorCode::ToolError,
            error_detail: "tool failed",
            is_retriable: Some(true),
            duration_ms: None,
        })
        .unwrap();
    let job = coord.job(&resp.job_id).unwrap();
    assert_eq!(job.error_code, Some(ErrorCode::ToolError));
    assert_eq!(job.is_retriable, Some(true));
    let end_event = coord
        .events()
        .iter()
        .find(|e| e.event_type == "job.end")
        .unwrap();
    assert_eq!(end_event.payload["error_code"], "tool_error");
}

#[test]
fn test_seq_numbers_monotonic() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 42, None).unwrap();
    coord
        .mark_succeeded(SuccessInfo {
            job_id: &resp.job_id,
            summary: "ok",
            artifacts: vec![],
            duration_ms: None,
        })
        .unwrap();
    let mut prev = 0u64;
    for e in coord.events() {
        assert!(e.seq > prev);
        prev = e.seq;
    }
}

#[test]
fn test_invalid_transition_returns_distinct_error() {
    let mut coord = test_coordinator();
    let resp = run_default(&mut coord);
    // Queued -> Running is invalid (must go through Preparing first)
    let err = coord.mark_ready(&resp.job_id, 42, None).unwrap_err();
    assert_eq!(err, CommandError::InvalidTransition);
}

// ── Issues 1, 4, 5: visibility / base_ref UX / cleanup_all ──────────────
// Tests moved to coding_coordinator_extra_tests.rs (file-size limit).
