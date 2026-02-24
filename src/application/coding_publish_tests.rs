use super::*;
use crate::domain::coding_ports::{
    CreatePrParams, GitHubPort, GitPrMutationResult, GitPrResult, GitPrStatusSummary, GitPushResult,
};

// ============================================================================
// Mock GitHub port
// ============================================================================

struct MockGitHub {
    push_ok: bool,
    push_error: Option<String>,
    branch_protected: bool,
    branch_protected_err: Option<String>,
    create_pr_ok: bool,
    create_pr_number: Option<u64>,
    create_pr_url: Option<String>,
    create_pr_error: Option<String>,
    mutation_ok: bool,
    mutation_error: Option<String>,
    pr_status_ok: bool,
    pr_status_error: Option<String>,
}

impl Default for MockGitHub {
    fn default() -> Self {
        Self {
            push_ok: true,
            push_error: None,
            branch_protected: false,
            branch_protected_err: None,
            create_pr_ok: true,
            create_pr_number: Some(42),
            create_pr_url: Some("https://github.com/org/repo/pull/42".to_string()),
            create_pr_error: None,
            mutation_ok: true,
            mutation_error: None,
            pr_status_ok: true,
            pr_status_error: None,
        }
    }
}

impl GitHubPort for MockGitHub {
    fn push_branch(&self, _repo: &str, _branch: &str, _force: bool) -> GitPushResult {
        GitPushResult {
            ok: self.push_ok,
            error: self.push_error.clone(),
        }
    }

    fn is_branch_protected(&self, _repo: &str, _branch: &str) -> Result<bool, String> {
        if let Some(err) = &self.branch_protected_err {
            return Err(err.clone());
        }
        Ok(self.branch_protected)
    }

    fn create_pr(&self, _params: &CreatePrParams) -> GitPrResult {
        GitPrResult {
            ok: self.create_pr_ok,
            pr_number: self.create_pr_number,
            url: self.create_pr_url.clone(),
            error: self.create_pr_error.clone(),
        }
    }

    fn update_pr(&self, _repo: &str, _pr: u64, _body: Option<&str>) -> GitPrMutationResult {
        GitPrMutationResult {
            ok: self.mutation_ok,
            error: self.mutation_error.clone(),
        }
    }

    fn request_review(&self, _repo: &str, _pr: u64, _reviewers: &[String]) -> GitPrMutationResult {
        GitPrMutationResult {
            ok: self.mutation_ok,
            error: self.mutation_error.clone(),
        }
    }

    fn add_labels(&self, _repo: &str, _pr: u64, _labels: &[String]) -> GitPrMutationResult {
        GitPrMutationResult {
            ok: self.mutation_ok,
            error: self.mutation_error.clone(),
        }
    }

    fn get_pr_status(&self, _repo: &str, _pr: u64) -> GitPrStatusSummary {
        GitPrStatusSummary {
            ok: self.pr_status_ok,
            state: Some("open".to_string()),
            review_state: Some("approved".to_string()),
            checks_passed: Some(true),
            error: self.pr_status_error.clone(),
        }
    }
}

fn default_job() -> PublishJobContext {
    PublishJobContext {
        job_id: "job_001".to_string(),
        run_id: "run_001".to_string(),
        state: JobState::Succeeded,
        repo: "org/repo".to_string(),
        branch: "quecto/job/job_001".to_string(),
    }
}

fn default_policy() -> GitHubPolicy {
    GitHubPolicy {
        side_effects_owner: "coordinator".to_string(),
        force_push_default: "deny".to_string(),
        destructive_reset_default: "deny".to_string(),
        repo_allowlist: Vec::new(),
        protected_branches: vec!["main".to_string(), "master".to_string()],
    }
}

// ============================================================================
// Policy tests
// ============================================================================

#[test]
fn test_default_policy_denies_force_push() {
    let policy = GitHubPolicy::default();
    assert_eq!(policy.force_push_default, "deny");
}

#[test]
fn test_default_policy_denies_destructive_reset() {
    let policy = GitHubPolicy::default();
    assert_eq!(policy.destructive_reset_default, "deny");
}

#[test]
fn test_default_policy_coordinator_owns_side_effects() {
    let policy = GitHubPolicy::default();
    assert_eq!(policy.side_effects_owner, "coordinator");
}

// ============================================================================
// Push branch tests
// ============================================================================

#[test]
fn test_push_branch_succeeds() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let req = PublishRequest::new("push_branch");
    let result = coord.publish(&req, &default_job());
    assert!(result.ok);
    assert_eq!(result.action, "push_branch");
}

#[test]
fn test_force_push_denied_by_policy() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let mut req = PublishRequest::new("push_branch");
    req.force = true;
    let result = coord.publish(&req, &default_job());
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("force-push is denied by policy"));
}

#[test]
fn test_push_to_protected_branch_denied() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let mut req = PublishRequest::new("push_branch");
    req.target_branch = Some("main".to_string());
    let result = coord.publish(&req, &default_job());
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("branch is protected"));
}

#[test]
fn test_push_to_protected_branch_with_api_confirmation() {
    let github = MockGitHub {
        branch_protected: true,
        ..MockGitHub::default()
    };
    let mut coord = PublishCoordinator::new(default_policy(), github);
    let mut req = PublishRequest::new("push_branch");
    req.target_branch = Some("main".to_string());
    let result = coord.publish(&req, &default_job());
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("branch protection rules"));
}

// ============================================================================
// PR tests
// ============================================================================

#[test]
fn test_create_pr_succeeds() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let mut req = PublishRequest::new("create_pr");
    req.title = Some("Add tests".to_string());
    req.base = Some("main".to_string());
    req.head = Some("quecto/job/job_001".to_string());
    let result = coord.publish(&req, &default_job());
    assert!(result.ok);
    assert!(result.pr_number.is_some());
    assert!(result.url.is_some());
}

#[test]
fn test_update_pr_succeeds() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    coord.set_existing_pr(123, "https://github.com/org/repo/pull/123");
    let mut req = PublishRequest::new("update_pr");
    req.body = Some("updated body".to_string());
    let result = coord.publish(&req, &default_job());
    assert!(result.ok);
}

#[test]
fn test_request_review_succeeds() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    coord.set_existing_pr(123, "https://github.com/org/repo/pull/123");
    let mut req = PublishRequest::new("request_review");
    req.reviewers = vec!["alice".to_string(), "bob".to_string()];
    let result = coord.publish(&req, &default_job());
    assert!(result.ok);
}

#[test]
fn test_add_labels_succeeds() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    coord.set_existing_pr(123, "https://github.com/org/repo/pull/123");
    let mut req = PublishRequest::new("add_labels");
    req.labels = vec!["automated".to_string()];
    let result = coord.publish(&req, &default_job());
    assert!(result.ok);
}

#[test]
fn test_get_pr_status_succeeds() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    coord.set_existing_pr(123, "https://github.com/org/repo/pull/123");
    let req = PublishRequest::new("get_pr_status");
    let result = coord.publish(&req, &default_job());
    assert!(result.ok);
}

// ============================================================================
// Job state validation tests
// ============================================================================

#[test]
fn test_publish_denied_for_failed_job() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let req = PublishRequest::new("create_pr");
    let mut job = default_job();
    job.state = JobState::Failed;
    let result = coord.publish(&req, &job);
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("did not succeed"));
}

#[test]
fn test_publish_denied_for_canceled_job() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let req = PublishRequest::new("push_branch");
    let mut job = default_job();
    job.state = JobState::Canceled;
    let result = coord.publish(&req, &job);
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("job was canceled"));
}

#[test]
fn test_get_pr_status_allowed_on_running_job() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    coord.set_existing_pr(123, "https://github.com/org/repo/pull/123");
    let req = PublishRequest::new("get_pr_status");
    let mut job = default_job();
    job.state = JobState::Running;
    let result = coord.publish(&req, &job);
    assert!(result.ok);
}

// ============================================================================
// Repo allowlist tests
// ============================================================================

#[test]
fn test_repo_not_in_allowlist_denied() {
    let mut policy = default_policy();
    policy.repo_allowlist = vec!["org/approved-repo".to_string()];
    let mut coord = PublishCoordinator::new(policy, MockGitHub::default());
    let mut req = PublishRequest::new("push_branch");
    req.target_repo = Some("org/forbidden-repo".to_string());
    let result = coord.publish(&req, &default_job());
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("allowlist"));
}

// ============================================================================
// Network error tests
// ============================================================================

#[test]
fn test_github_timeout_handled() {
    let github = MockGitHub {
        push_ok: false,
        push_error: Some("timeout".to_string()),
        ..MockGitHub::default()
    };
    let mut coord = PublishCoordinator::new(default_policy(), github);
    let req = PublishRequest::new("push_branch");
    let result = coord.publish(&req, &default_job());
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("network timeout"));
}

#[test]
fn test_github_rate_limit_handled() {
    let github = MockGitHub {
        push_ok: false,
        push_error: Some("rate limit".to_string()),
        ..MockGitHub::default()
    };
    let mut coord = PublishCoordinator::new(default_policy(), github);
    let req = PublishRequest::new("push_branch");
    let result = coord.publish(&req, &default_job());
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("rate limiting"));
}

// ============================================================================
// Destructive reset tests
// ============================================================================

#[test]
fn test_destructive_reset_denied_by_default() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let req = PublishRequest::new("destructive_reset");
    let result = coord.publish(&req, &default_job());
    assert!(!result.ok);
    let err = result.error.unwrap();
    assert!(err.contains("destructive_reset_default"));
}

// ============================================================================
// Source validation tests
// ============================================================================

#[test]
fn test_worker_source_rejected() {
    let coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let result = coord.validate_source_is_coordinator("worker");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), PublishError::CoordinatorOnly);
}

#[test]
fn test_coordinator_source_accepted() {
    let coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let result = coord.validate_source_is_coordinator("coordinator");
    assert!(result.is_ok());
}

// ============================================================================
// Credential scoping tests
// ============================================================================

#[test]
fn test_worker_forbidden_env_keys_not_empty() {
    let keys = worker_forbidden_env_keys();
    assert!(!keys.is_empty());
    assert!(keys.contains(&"GITHUB_TOKEN"));
}

#[test]
fn test_coordinator_holds_credentials_when_coordinator_owns() {
    let policy = default_policy();
    assert!(coordinator_holds_credentials(&policy));
}

#[test]
fn test_coordinator_does_not_hold_when_worker_owns() {
    let mut policy = default_policy();
    policy.side_effects_owner = "worker".to_string();
    assert!(!coordinator_holds_credentials(&policy));
}

// ============================================================================
// Contract fidelity tests
// ============================================================================

#[test]
fn test_result_action_matches_request_action() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let mut req = PublishRequest::new("create_pr");
    req.title = Some("test".to_string());
    let result = coord.publish(&req, &default_job());
    assert_eq!(result.action, "create_pr");
}

#[test]
fn test_error_result_still_has_correct_action() {
    let mut coord = PublishCoordinator::new(default_policy(), MockGitHub::default());
    let mut req = PublishRequest::new("push_branch");
    req.force = true;
    let result = coord.publish(&req, &default_job());
    assert_eq!(result.action, "push_branch");
    assert!(!result.ok);
}

// ============================================================================
// can_publish_for_state tests
// ============================================================================

#[test]
fn test_can_publish_succeeded() {
    assert!(
        PublishCoordinator::<MockGitHub>::can_publish_for_state("create_pr", JobState::Succeeded)
            .is_ok()
    );
}

#[test]
fn test_can_publish_failed_denied() {
    let err =
        PublishCoordinator::<MockGitHub>::can_publish_for_state("create_pr", JobState::Failed)
            .unwrap_err();
    assert_eq!(
        err,
        PublishError::InvalidJobState("job did not succeed".to_string())
    );
}

#[test]
fn test_can_publish_running_denied() {
    let err =
        PublishCoordinator::<MockGitHub>::can_publish_for_state("push_branch", JobState::Running)
            .unwrap_err();
    assert_eq!(
        err,
        PublishError::InvalidJobState("job is not in a publishable terminal state".to_string())
    );
}

#[test]
fn test_get_pr_status_allowed_any_state() {
    for state in [
        JobState::Running,
        JobState::Failed,
        JobState::Canceled,
        JobState::Queued,
    ] {
        assert!(
            PublishCoordinator::<MockGitHub>::can_publish_for_state("get_pr_status", state).is_ok()
        );
    }
}
