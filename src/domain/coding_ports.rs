//! Port traits for coding job coordination.
//!
//! These define what the application layer needs from the outside world.
//! Infrastructure adapters implement these traits.

/// Port for validating repository and ref existence.
pub trait RepoValidator {
    fn repo_exists(&self, repo: &str) -> bool;
    fn ref_exists(&self, repo: &str, base_ref: &str) -> bool;
}

/// Port for resolving skills from the workspace.
pub trait SkillResolver {
    fn skill_exists(&self, name: &str) -> bool;
}

/// Port for checking whether an OS process is still alive.
pub trait ProcessChecker {
    fn is_alive(&self, pid: u32) -> bool;
}

/// A single line result from reading an event log.
#[derive(Debug, Clone)]
pub enum EventLogLine {
    /// Successfully parsed event envelope.
    Valid(super::coding_event::EventEnvelope),
    /// Corrupted or truncated line that could not be parsed.
    Corrupt { line_number: usize, raw: String },
}

/// Port for reading persisted JSONL event logs.
///
/// Each job has its own event log file. The reader discovers job
/// directories and returns their event lines.
pub trait EventLogStore {
    /// Returns the list of job IDs that have event log directories.
    fn discover_jobs(&self) -> Vec<String>;

    /// Reads all lines from the event log for the given job.
    fn read_log(&self, job_id: &str) -> Vec<EventLogLine>;

    /// Appends an event envelope to the log for the given job.
    fn append_event(&mut self, job_id: &str, event: &super::coding_event::EventEnvelope);

    /// Writes or overwrites the jobs index from recovered state.
    fn write_index(&mut self, entries: &[(String, super::coding_job::JobState)]);

    /// Attempts to acquire the coordinator lock. Returns `true` if
    /// acquired, `false` if already held by another process.
    fn try_acquire_lock(&self) -> bool;
}

// ============================================================================
// Coding job service port (used by tool layer)
// ============================================================================

use super::coding_command::{
    CancelResponse, CleanupResponse, CommandError, ListRequest, ListResponse, RunRequest,
    RunResponse, StatusResponse,
};

/// Port for coding job management operations.
///
/// The agent's coding_job tool depends on this trait rather than importing
/// the application-layer coordinator directly. The coordinator implements
/// this trait so the tool stays in infrastructure without violating the
/// dependency rule (infrastructure → domain only, never → application).
pub trait CodingJobService: Send {
    /// Start a new coding job.
    fn run(&mut self, req: RunRequest) -> Result<RunResponse, CommandError>;

    /// Query status by job ID.
    fn status_by_job_id(&self, job_id: &str) -> Result<StatusResponse, CommandError>;

    /// Query status by run ID.
    fn status_by_run_id(&self, run_id: &str) -> Result<StatusResponse, CommandError>;

    /// Cancel a job.
    fn cancel(&mut self, job_id: &str) -> Result<CancelResponse, CommandError>;

    /// Clean up a terminated job's artifacts.
    fn cleanup(
        &mut self,
        job_id: &str,
        keep_artifacts: bool,
    ) -> Result<CleanupResponse, CommandError>;

    /// List jobs, optionally filtered by state.
    fn list(&self, req: &ListRequest) -> ListResponse;
}

// ============================================================================
// GitHub publish ports
// ============================================================================

/// Result of a GitHub push operation.
#[derive(Debug, Clone)]
pub struct GitPushResult {
    pub ok: bool,
    pub error: Option<String>,
}

/// Result of a GitHub PR creation operation.
#[derive(Debug, Clone)]
pub struct GitPrResult {
    pub ok: bool,
    pub pr_number: Option<u64>,
    pub url: Option<String>,
    pub error: Option<String>,
}

/// Result of a generic GitHub PR mutation (update, add labels, request review).
#[derive(Debug, Clone)]
pub struct GitPrMutationResult {
    pub ok: bool,
    pub error: Option<String>,
}

/// PR status summary returned by the GitHub port.
#[derive(Debug, Clone)]
pub struct GitPrStatusSummary {
    pub ok: bool,
    pub state: Option<String>,
    pub review_state: Option<String>,
    pub checks_passed: Option<bool>,
    pub error: Option<String>,
}

/// Port for GitHub API operations used by publish coordination.
///
/// All methods are synchronous for simplicity — the application layer
/// is synchronous; real I/O adapters can block internally.
pub trait GitHubPort {
    /// Push a branch to the remote. `force` indicates force-push.
    fn push_branch(&self, repo: &str, branch: &str, force: bool) -> GitPushResult;

    /// Check if a branch is protected on the remote.
    fn is_branch_protected(&self, repo: &str, branch: &str) -> Result<bool, String>;

    /// Create a pull request.
    fn create_pr(&self, params: &CreatePrParams) -> GitPrResult;

    /// Update an existing pull request body or title.
    fn update_pr(&self, repo: &str, pr_number: u64, body: Option<&str>) -> GitPrMutationResult;

    /// Request reviewers on a pull request.
    fn request_review(
        &self,
        repo: &str,
        pr_number: u64,
        reviewers: &[String],
    ) -> GitPrMutationResult;

    /// Add labels to a pull request.
    fn add_labels(&self, repo: &str, pr_number: u64, labels: &[String]) -> GitPrMutationResult;

    /// Get status and review summary for a pull request.
    fn get_pr_status(&self, repo: &str, pr_number: u64) -> GitPrStatusSummary;
}

/// Parameters for creating a pull request via `GitHubPort`.
#[derive(Debug, Clone)]
pub struct CreatePrParams {
    pub repo: String,
    pub title: String,
    pub base: String,
    pub head: String,
    pub body: Option<String>,
}
