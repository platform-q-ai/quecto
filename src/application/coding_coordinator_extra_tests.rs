//! Extra coordinator tests: Issues 1 (status visibility), 4 (base_ref UX),
//! 5 (cleanup_all). Split from coding_coordinator_tests.rs to stay under the
//! 750-line source file limit.

use super::*;
use crate::domain::coding_job::Priority;

// ── Shared helpers (mirrors coding_coordinator_tests.rs) ─────────────────

struct MockRepoValidator2 {
    valid_repos: Vec<String>,
    valid_refs: Vec<(String, String)>,
}

impl RepoValidator for MockRepoValidator2 {
    fn repo_exists(&self, repo: &str) -> bool {
        self.valid_repos.iter().any(|r| r == repo)
    }
    fn ref_exists(&self, repo: &str, base_ref: &str) -> bool {
        self.valid_refs
            .iter()
            .any(|(r, b)| r == repo && b == base_ref)
    }
}

struct MockSkillResolver2 {
    available: Vec<String>,
}

impl SkillResolver for MockSkillResolver2 {
    fn skill_exists(&self, name: &str) -> bool {
        self.available.iter().any(|s| s == name)
    }
}

fn make_coord() -> CodingCoordinator<MockRepoValidator2, MockSkillResolver2> {
    CodingCoordinator::new(
        MockRepoValidator2 {
            valid_repos: vec!["test-repo".to_string()],
            valid_refs: vec![("test-repo".to_string(), "main".to_string())],
        },
        MockSkillResolver2 {
            available: vec!["rust-style".to_string()],
        },
        CoordinatorPolicy::default(),
    )
}

fn run_one(coord: &mut CodingCoordinator<MockRepoValidator2, MockSkillResolver2>) -> RunResponse {
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

// ── Issue 1: status visibility ───────────────────────────────────────────

#[test]
fn test_status_includes_created_at() {
    let mut coord = make_coord();
    let resp = run_one(&mut coord);
    let status = coord.status_by_job_id(&resp.job_id).unwrap();
    assert!(
        status.created_at.is_some(),
        "created_at must be set on a new job"
    );
    assert!(status.created_at.unwrap() > 0);
}

#[test]
fn test_status_includes_state_entered_at() {
    let mut coord = make_coord();
    let resp = run_one(&mut coord);
    let status = coord.status_by_job_id(&resp.job_id).unwrap();
    assert!(
        status.state_entered_at.is_some(),
        "state_entered_at must be set"
    );
    assert!(status.state_entered_at.unwrap() > 0);
}

#[test]
fn test_status_last_event_fields_start_none() {
    let mut coord = make_coord();
    let resp = run_one(&mut coord);
    let status = coord.status_by_job_id(&resp.job_id).unwrap();
    assert!(
        status.last_event_ts.is_none(),
        "no events yet for a freshly queued job"
    );
    assert!(status.last_event_type.is_none());
}

#[test]
fn test_status_last_event_populated_after_emit() {
    let mut coord = make_coord();
    let resp = run_one(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    let status = coord.status_by_job_id(&resp.job_id).unwrap();
    assert!(
        status.last_event_ts.is_some(),
        "begin_preparation emits job.start → last_event_ts should be set"
    );
    assert_eq!(status.last_event_type.as_deref(), Some("job.start"));
}

#[test]
fn test_status_last_event_updated_on_receive_event() {
    let mut coord = make_coord();
    let resp = run_one(&mut coord);
    coord.begin_preparation(&resp.job_id).unwrap();
    coord.mark_ready(&resp.job_id, 99, None).unwrap();

    let envelope = crate::domain::coding_event::EventEnvelope {
        v: "1.0".to_string(),
        ts: "2026-01-01T12:00:00Z".to_string(),
        run_id: resp.run_id.clone(),
        job_id: resp.job_id.clone(),
        source: crate::domain::coding_event::EventSource::Worker,
        event_type: "log.message".to_string(),
        seq: 99,
        payload: serde_json::json!({"level":"info","message":"hello"}),
    };
    coord.receive_event(envelope).unwrap();

    let status = coord.status_by_job_id(&resp.job_id).unwrap();
    assert_eq!(status.last_event_type.as_deref(), Some("log.message"));
    assert_eq!(
        status.last_event_ts.as_deref(),
        Some("2026-01-01T12:00:00Z")
    );
}

#[test]
fn test_list_includes_metadata_fields() {
    let mut coord = make_coord();
    run_one(&mut coord);
    let list = coord.list(&crate::domain::coding_command::ListRequest { state_filter: None });
    assert_eq!(list.jobs.len(), 1);
    let entry = &list.jobs[0];
    assert!(entry.created_at.is_some());
    assert!(entry.state_entered_at.is_some());
    assert!(entry.last_event_ts.is_none());
    assert!(entry.last_event_type.is_none());
}

#[test]
fn test_state_entered_at_advances_on_transition() {
    let mut coord = make_coord();
    let resp = run_one(&mut coord);
    let before = coord
        .status_by_job_id(&resp.job_id)
        .unwrap()
        .state_entered_at
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    coord.begin_preparation(&resp.job_id).unwrap();
    let after = coord
        .status_by_job_id(&resp.job_id)
        .unwrap()
        .state_entered_at
        .unwrap();
    assert!(
        after >= before,
        "state_entered_at must not decrease: before={before} after={after}"
    );
}

// ── Issue 4: base_ref UX ─────────────────────────────────────────────────

#[test]
fn test_invalid_base_ref_returns_detail_variant() {
    let mut coord = make_coord();
    let err = coord
        .run(RunRequest {
            goal: "g".to_string(),
            repo: "test-repo".to_string(),
            base_ref: "no-such-branch".to_string(),
            priority: Priority::default(),
            profile: "default".to_string(),
            max_wall_seconds: None,
            labels: vec![],
            skills: vec![],
        })
        .unwrap_err();
    assert!(
        matches!(err, CommandError::InvalidBaseRefDetail(_)),
        "expected InvalidBaseRefDetail, got {err:?}"
    );
}

#[test]
fn test_invalid_base_ref_detail_contains_default_branch_hint() {
    let mut coord = make_coord();
    let err = coord
        .run(RunRequest {
            goal: "g".to_string(),
            repo: "test-repo".to_string(),
            base_ref: "no-such-branch".to_string(),
            priority: Priority::default(),
            profile: "default".to_string(),
            max_wall_seconds: None,
            labels: vec![],
            skills: vec![],
        })
        .unwrap_err();
    let detail = err.to_string();
    assert!(
        detail.starts_with("invalid_base_ref: "),
        "display must start with 'invalid_base_ref: ', got: {detail}"
    );
    assert!(
        detail.contains("default_branch="),
        "detail must include default_branch hint, got: {detail}"
    );
    assert!(
        detail.contains("available_refs="),
        "detail must include available_refs hint, got: {detail}"
    );
}

#[test]
fn test_invalid_base_ref_detail_parse_round_trip() {
    let original = CommandError::InvalidBaseRefDetail(
        "default_branch=main; available_refs=[main, dev]".to_string(),
    );
    let s = original.to_string();
    let parsed: CommandError = s.parse().unwrap();
    assert!(
        matches!(parsed, CommandError::InvalidBaseRefDetail(_)),
        "parse round-trip failed: {parsed:?}"
    );
}

// ── Issue 5: cleanup_all ─────────────────────────────────────────────────

#[test]
fn test_cleanup_all_removes_terminal_jobs() {
    let mut coord = make_coord();
    let r1 = run_one(&mut coord);
    let r2 = run_one(&mut coord);
    coord.cancel(&r1.job_id).unwrap();
    coord.cancel(&r2.job_id).unwrap();

    let result = coord
        .cleanup_all_impl(&crate::domain::coding_command::CleanupAllRequest {
            state_filter: None,
            keep_artifacts: true,
            terminal_only: true,
        })
        .unwrap();

    assert_eq!(result.cleaned_count, 2);
    assert_eq!(result.skipped_job_ids.len(), 0);
    assert!(coord.job(&r1.job_id).is_none());
    assert!(coord.job(&r2.job_id).is_none());
}

#[test]
fn test_cleanup_all_skips_non_terminal_when_terminal_only() {
    let mut coord = make_coord();
    let r_running = run_one(&mut coord);
    let r_canceled = run_one(&mut coord);
    coord.cancel(&r_canceled.job_id).unwrap();

    let result = coord
        .cleanup_all_impl(&crate::domain::coding_command::CleanupAllRequest {
            state_filter: None,
            keep_artifacts: true,
            terminal_only: true,
        })
        .unwrap();

    assert_eq!(result.cleaned_count, 1);
    assert_eq!(result.skipped_job_ids.len(), 1);
    assert!(coord.job(&r_canceled.job_id).is_none());
    assert!(coord.job(&r_running.job_id).is_some());
}

#[test]
fn test_cleanup_all_errors_on_non_terminal_when_not_terminal_only() {
    let mut coord = make_coord();
    run_one(&mut coord);

    let err = coord
        .cleanup_all_impl(&crate::domain::coding_command::CleanupAllRequest {
            state_filter: None,
            keep_artifacts: true,
            terminal_only: false,
        })
        .unwrap_err();

    assert_eq!(err, CommandError::JobNotTerminal);
}

#[test]
fn test_cleanup_all_state_filter_only_removes_matching() {
    let mut coord = make_coord();
    let r_canceled = run_one(&mut coord);
    let r_succeeded = run_one(&mut coord);
    coord.cancel(&r_canceled.job_id).unwrap();
    coord.begin_preparation(&r_succeeded.job_id).unwrap();
    coord.mark_ready(&r_succeeded.job_id, 1, None).unwrap();
    coord
        .mark_succeeded(SuccessInfo {
            job_id: &r_succeeded.job_id,
            summary: "done",
            artifacts: vec![],
            duration_ms: None,
        })
        .unwrap();

    let result = coord
        .cleanup_all_impl(&crate::domain::coding_command::CleanupAllRequest {
            state_filter: Some(vec![crate::domain::coding_job::JobState::Succeeded]),
            keep_artifacts: true,
            terminal_only: true,
        })
        .unwrap();

    assert_eq!(result.cleaned_count, 1);
    assert!(coord.job(&r_succeeded.job_id).is_none());
    assert!(coord.job(&r_canceled.job_id).is_some());
}
