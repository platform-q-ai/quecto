//! Publish coordination — manages GitHub publish operations under policy control.
//!
//! The publish coordinator enforces safety gates (no force-push, protected
//! branch detection, repo allowlist, credential scoping) and emits
//! `publish.request` / `publish.result` event pairs for every operation.

use crate::domain::coding_job::JobState;
use crate::domain::coding_ports::{CreatePrParams, GitHubPort};

// ============================================================================
// Policy
// ============================================================================

/// GitHub publish policy controlling what the coordinator allows.
#[derive(Debug, Clone)]
pub struct GitHubPolicy {
    /// Who owns side effects: "coordinator" or "worker".
    pub side_effects_owner: String,
    /// Default for force-push: "deny" or "allow".
    pub force_push_default: String,
    /// Default for destructive resets: "deny" or "allow".
    pub destructive_reset_default: String,
    /// Repos allowed for publish; empty means all allowed.
    pub repo_allowlist: Vec<String>,
    /// Branches considered protected (local static list).
    pub protected_branches: Vec<String>,
}

impl Default for GitHubPolicy {
    fn default() -> Self {
        Self {
            side_effects_owner: "coordinator".to_string(),
            force_push_default: "deny".to_string(),
            destructive_reset_default: "deny".to_string(),
            repo_allowlist: Vec::new(),
            protected_branches: vec!["main".to_string(), "master".to_string()],
        }
    }
}

// ============================================================================
// Publish request / result types
// ============================================================================

/// A publish action request from the main agent.
#[derive(Debug, Clone)]
pub struct PublishRequest {
    pub action: String,
    pub force: bool,
    pub target_branch: Option<String>,
    pub title: Option<String>,
    pub base: Option<String>,
    pub head: Option<String>,
    pub body: Option<String>,
    pub labels: Vec<String>,
    pub reviewers: Vec<String>,
    pub target_repo: Option<String>,
}

impl PublishRequest {
    /// Create a simple request with just an action name.
    pub fn new(action: &str) -> Self {
        Self {
            action: action.to_string(),
            force: false,
            target_branch: None,
            title: None,
            base: None,
            head: None,
            body: None,
            labels: Vec::new(),
            reviewers: Vec::new(),
            target_repo: None,
        }
    }
}

/// The result of a publish operation.
#[derive(Debug, Clone)]
pub struct PublishResult {
    pub action: String,
    pub ok: bool,
    pub error: Option<String>,
    pub pr_number: Option<u64>,
    pub url: Option<String>,
}

// ============================================================================
// Publish error
// ============================================================================

/// Errors specific to publish operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// Caller is not the coordinator (e.g. worker tried to publish).
    CoordinatorOnly,
    /// Job state does not permit this publish action.
    InvalidJobState(String),
    /// Force-push denied by policy.
    ForcePushDenied,
    /// Target branch is protected.
    BranchProtected(String),
    /// Destructive reset denied by policy.
    DestructiveResetDenied,
    /// Target repo not in allowlist.
    RepoNotAllowed(String),
    /// Network timeout from GitHub API.
    NetworkTimeout,
    /// Rate limited by GitHub API.
    RateLimited,
    /// Branch protection detected from GitHub API.
    BranchProtectionRules(String),
    /// Generic GitHub API error.
    GitHubError(String),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoordinatorOnly => write!(f, "publish is coordinator-only"),
            Self::InvalidJobState(msg) => write!(f, "{msg}"),
            Self::ForcePushDenied => write!(f, "force-push is denied by policy"),
            Self::BranchProtected(b) => write!(f, "branch is protected: {b}"),
            Self::DestructiveResetDenied => {
                write!(f, "destructive_reset_default policy")
            }
            Self::RepoNotAllowed(r) => {
                write!(f, "repo is not in the allowlist: {r}")
            }
            Self::NetworkTimeout => write!(f, "network timeout"),
            Self::RateLimited => write!(f, "rate limiting"),
            Self::BranchProtectionRules(b) => {
                write!(f, "branch protection rules: {b}")
            }
            Self::GitHubError(e) => write!(f, "github error: {e}"),
        }
    }
}

// ============================================================================
// Publish coordinator
// ============================================================================

/// Coordinates GitHub publish operations under policy control.
pub struct PublishCoordinator<G: GitHubPort> {
    policy: GitHubPolicy,
    github: G,
    /// Existing PR number for the current job (set after create_pr or given).
    pr_number: Option<u64>,
    /// PR url for the current job.
    pr_url: Option<String>,
}

impl<G: GitHubPort> std::fmt::Debug for PublishCoordinator<G> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishCoordinator")
            .field("pr_number", &self.pr_number)
            .finish()
    }
}

impl<G: GitHubPort> PublishCoordinator<G> {
    pub fn new(policy: GitHubPolicy, github: G) -> Self {
        Self {
            policy,
            github,
            pr_number: None,
            pr_url: None,
        }
    }

    pub fn policy(&self) -> &GitHubPolicy {
        &self.policy
    }

    /// Set an existing PR for the job (for "given a PR exists" scenarios).
    pub fn set_existing_pr(&mut self, pr_number: u64, url: &str) {
        self.pr_number = Some(pr_number);
        self.pr_url = Some(url.to_string());
    }

    pub fn pr_number(&self) -> Option<u64> {
        self.pr_number
    }

    /// Execute a publish action, returning a result.
    pub fn publish(&mut self, req: &PublishRequest, job: &PublishJobContext) -> PublishResult {
        match self.execute_publish(req, job) {
            Ok(result) => result,
            Err(err) => PublishResult {
                action: req.action.clone(),
                ok: false,
                error: Some(err.to_string()),
                pr_number: None,
                url: None,
            },
        }
    }

    /// Validate that a worker source is rejected for publish.
    pub fn validate_source_is_coordinator(&self, source: &str) -> Result<(), PublishError> {
        if source == "worker" {
            return Err(PublishError::CoordinatorOnly);
        }
        Ok(())
    }

    /// Check whether the given job state allows publish for the action.
    pub fn can_publish_for_state(action: &str, state: JobState) -> Result<(), PublishError> {
        // Read-only queries are allowed on any state.
        if action == "get_pr_status" {
            return Ok(());
        }
        if state == JobState::Succeeded {
            return Ok(());
        }
        let msg = match state {
            JobState::Failed => "job did not succeed",
            JobState::Canceled => "job was canceled",
            _ => "job is not in a publishable terminal state",
        };
        Err(PublishError::InvalidJobState(msg.to_string()))
    }

    fn execute_publish(
        &mut self,
        req: &PublishRequest,
        job: &PublishJobContext,
    ) -> Result<PublishResult, PublishError> {
        // Job state validation
        Self::can_publish_for_state(&req.action, job.state)?;

        // Repo allowlist check
        let repo = req.target_repo.as_deref().unwrap_or(&job.repo);
        self.check_repo_allowlist(repo)?;

        // Dispatch by action
        match req.action.as_str() {
            "push_branch" => self.handle_push_branch(req, job),
            "create_pr" => self.handle_create_pr(req, job),
            "update_pr" => self.handle_update_pr(req, job),
            "request_review" => self.handle_request_review(req, job),
            "add_labels" => self.handle_add_labels(req, job),
            "get_pr_status" => self.handle_get_pr_status(job),
            "destructive_reset" => self.handle_destructive_reset(),
            _ => Ok(PublishResult {
                action: req.action.clone(),
                ok: false,
                error: Some(format!("unknown action: {}", req.action)),
                pr_number: None,
                url: None,
            }),
        }
    }

    fn check_repo_allowlist(&self, repo: &str) -> Result<(), PublishError> {
        if self.policy.repo_allowlist.is_empty() {
            return Ok(());
        }
        if self.policy.repo_allowlist.iter().any(|r| r == repo) {
            return Ok(());
        }
        Err(PublishError::RepoNotAllowed(repo.to_string()))
    }

    fn handle_push_branch(
        &self,
        req: &PublishRequest,
        job: &PublishJobContext,
    ) -> Result<PublishResult, PublishError> {
        // Force-push policy check
        if req.force && self.policy.force_push_default == "deny" {
            return Err(PublishError::ForcePushDenied);
        }

        let branch = req.target_branch.as_deref().unwrap_or(&job.branch);

        // Static protected branch check
        if self.policy.protected_branches.contains(&branch.to_string()) {
            // Also check the remote API for protection status
            match self.github.is_branch_protected(&job.repo, branch) {
                Ok(true) => {
                    return Err(PublishError::BranchProtectionRules(branch.to_string()));
                }
                Ok(false) => {
                    // Static list says protected but API says no —
                    // trust the static list for safety.
                    return Err(PublishError::BranchProtected(branch.to_string()));
                }
                Err(e) if e.contains("timeout") => {
                    return Err(PublishError::NetworkTimeout);
                }
                Err(e) if e.contains("rate limit") => {
                    return Err(PublishError::RateLimited);
                }
                Err(_) => {
                    return Err(PublishError::BranchProtected(branch.to_string()));
                }
            }
        }

        // Execute push via port
        let result = self.github.push_branch(&job.repo, branch, req.force);
        if !result.ok {
            let err = result.error.unwrap_or_default();
            if err.contains("timeout") {
                return Err(PublishError::NetworkTimeout);
            }
            if err.contains("rate limit") {
                return Err(PublishError::RateLimited);
            }
            return Err(PublishError::GitHubError(err));
        }

        Ok(PublishResult {
            action: req.action.clone(),
            ok: true,
            error: None,
            pr_number: None,
            url: None,
        })
    }

    fn handle_create_pr(
        &mut self,
        req: &PublishRequest,
        job: &PublishJobContext,
    ) -> Result<PublishResult, PublishError> {
        let params = CreatePrParams {
            repo: job.repo.clone(),
            title: req.title.clone().unwrap_or_default(),
            base: req.base.clone().unwrap_or_else(|| "main".to_string()),
            head: req.head.clone().unwrap_or_else(|| job.branch.clone()),
            body: req.body.clone(),
        };
        let result = self.github.create_pr(&params);
        if !result.ok {
            let err = result.error.unwrap_or_default();
            if err.contains("timeout") {
                return Err(PublishError::NetworkTimeout);
            }
            if err.contains("rate limit") {
                return Err(PublishError::RateLimited);
            }
            return Err(PublishError::GitHubError(err));
        }
        self.pr_number = result.pr_number;
        self.pr_url = result.url.clone();
        Ok(PublishResult {
            action: req.action.clone(),
            ok: true,
            error: None,
            pr_number: result.pr_number,
            url: result.url,
        })
    }

    fn handle_update_pr(
        &self,
        req: &PublishRequest,
        job: &PublishJobContext,
    ) -> Result<PublishResult, PublishError> {
        let pr = self
            .pr_number
            .ok_or_else(|| PublishError::GitHubError("no PR exists".to_string()))?;
        let result = self.github.update_pr(&job.repo, pr, req.body.as_deref());
        if !result.ok {
            return Err(PublishError::GitHubError(result.error.unwrap_or_default()));
        }
        Ok(PublishResult {
            action: req.action.clone(),
            ok: true,
            error: None,
            pr_number: self.pr_number,
            url: self.pr_url.clone(),
        })
    }

    fn handle_request_review(
        &self,
        req: &PublishRequest,
        job: &PublishJobContext,
    ) -> Result<PublishResult, PublishError> {
        let pr = self
            .pr_number
            .ok_or_else(|| PublishError::GitHubError("no PR exists".to_string()))?;
        let result = self.github.request_review(&job.repo, pr, &req.reviewers);
        if !result.ok {
            return Err(PublishError::GitHubError(result.error.unwrap_or_default()));
        }
        Ok(PublishResult {
            action: req.action.clone(),
            ok: true,
            error: None,
            pr_number: self.pr_number,
            url: self.pr_url.clone(),
        })
    }

    fn handle_add_labels(
        &self,
        req: &PublishRequest,
        job: &PublishJobContext,
    ) -> Result<PublishResult, PublishError> {
        let pr = self
            .pr_number
            .ok_or_else(|| PublishError::GitHubError("no PR exists".to_string()))?;
        let result = self.github.add_labels(&job.repo, pr, &req.labels);
        if !result.ok {
            return Err(PublishError::GitHubError(result.error.unwrap_or_default()));
        }
        Ok(PublishResult {
            action: req.action.clone(),
            ok: true,
            error: None,
            pr_number: self.pr_number,
            url: self.pr_url.clone(),
        })
    }

    fn handle_get_pr_status(&self, job: &PublishJobContext) -> Result<PublishResult, PublishError> {
        // get_pr_status is allowed even without an existing PR number —
        // it may query by branch. Use pr_number if we have it, otherwise
        // return a stub summary.
        if let Some(pr) = self.pr_number {
            let summary = self.github.get_pr_status(&job.repo, pr);
            if !summary.ok {
                let err = summary.error.unwrap_or_default();
                if err.contains("timeout") {
                    return Err(PublishError::NetworkTimeout);
                }
                return Err(PublishError::GitHubError(err));
            }
        }
        Ok(PublishResult {
            action: "get_pr_status".to_string(),
            ok: true,
            error: None,
            pr_number: self.pr_number,
            url: self.pr_url.clone(),
        })
    }

    fn handle_destructive_reset(&self) -> Result<PublishResult, PublishError> {
        if self.policy.destructive_reset_default == "deny" {
            return Err(PublishError::DestructiveResetDenied);
        }
        Ok(PublishResult {
            action: "destructive_reset".to_string(),
            ok: true,
            error: None,
            pr_number: None,
            url: None,
        })
    }
}

/// Minimal job context needed by the publish coordinator.
#[derive(Debug, Clone)]
pub struct PublishJobContext {
    pub job_id: String,
    pub run_id: String,
    pub state: JobState,
    pub repo: String,
    pub branch: String,
}

// ============================================================================
// Credential scoping
// ============================================================================

/// Returns the set of environment variable names that should NOT
/// be present in a worker process for credential scoping.
pub fn worker_forbidden_env_keys() -> &'static [&'static str] {
    &["GITHUB_TOKEN", "GH_TOKEN", "GITHUB_PAT"]
}

/// Returns true if the coordinator (not the worker) should hold
/// GitHub credentials.
pub fn coordinator_holds_credentials(policy: &GitHubPolicy) -> bool {
    policy.side_effects_owner == "coordinator"
}

#[cfg(test)]
#[path = "coding_publish_tests.rs"]
mod tests;
