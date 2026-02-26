//! Port traits for coding job coordination.
//!
//! These define what the application layer needs from the outside world.
//! Infrastructure adapters implement these traits.

/// Port for emitting structured worker events.
///
/// The application layer calls `emit()` to produce events. Infrastructure
/// provides a concrete implementation (e.g. JSON Lines to stdout).
/// Uses `&self` (not `&mut self`) following the project's port-trait
/// convention — implementations handle interior mutability.
pub trait WorkerEventSink: Send + Sync {
    /// Emit an event with the given type and JSON payload.
    /// Returns the sequence number on success.
    fn emit(&self, event_type: &str, payload: serde_json::Value) -> Result<u64, String>;
}

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
// Repo mirror and per-job clone port
// ============================================================================

/// Result of a mirror-fetch or clone operation.
#[derive(Debug, Clone)]
pub struct RepoOpResult {
    /// Whether the operation succeeded.
    pub ok: bool,
    /// Duration of the operation in milliseconds.
    pub duration_ms: u64,
    /// Error message on failure.
    pub error: Option<String>,
    /// Error classification for coordinator state transitions.
    pub error_code: Option<String>,
}

/// Parameters for cloning a repo for a specific job.
#[derive(Debug, Clone)]
pub struct CloneJobParams<'a> {
    pub repo: &'a str,
    pub job_id: &'a str,
    pub base_ref: &'a str,
    pub job_branch: &'a str,
}

/// Port for bare mirror cache and per-job clone operations.
///
/// The coordinator uses this trait to create/update mirrors and clone
/// repositories for coding jobs. Each implementation handles git operations,
/// flock coordination, and path safety.
pub trait RepoMirrorStore {
    /// Check whether a bare mirror exists for the given repo identifier.
    fn mirror_exists(&self, repo: &str) -> bool;

    /// Create a new bare mirror for the repo. Returns error if invalid path.
    fn create_mirror(&mut self, repo: &str, remote_url: &str) -> RepoOpResult;

    /// Fetch latest refs into an existing mirror (acquires exclusive flock).
    fn fetch_mirror(&self, repo: &str) -> RepoOpResult;

    /// Clone from the local mirror into a job directory (acquires shared flock).
    fn clone_for_job(&self, params: &CloneJobParams<'_>) -> RepoOpResult;

    /// Convert a repo identifier to a safe mirror directory name.
    /// Returns `None` if the repo contains path traversal or invalid characters.
    fn mirror_path_for_repo(&self, repo: &str) -> Option<String>;

    /// Remove a job's cloned repo directory.
    fn remove_job_repo(&self, job_id: &str) -> bool;

    /// Remove a job's repo directory but preserve its artifact directory.
    fn remove_job_repo_keep_artifacts(&self, job_id: &str) -> bool;

    /// Return the absolute path to the cloned repo directory for a job.
    ///
    /// This allows the application layer to pass the correct `job_dir`
    /// to `WorkerLaunchConfig` without hardcoding filesystem paths.
    fn job_repo_path(&self, job_id: &str) -> String;
}

// ============================================================================
// Worker runtime port
// ============================================================================

/// Configuration for launching an nsjail worker process.
#[derive(Debug, Clone)]
pub struct WorkerLaunchConfig {
    /// Path to the job directory (sole writable mount).
    pub job_dir: String,
    /// Goal description for the worker.
    pub goal: String,
    /// Resource limits.
    pub max_memory_mb: u32,
    pub max_cpu_seconds: u32,
    pub max_wall_seconds: u32,
    pub max_pids: u32,
    /// Network policy: "deny" (default) or list of allowed hosts.
    pub network_allowed_hosts: Vec<String>,
    /// Whether die-with-parent is enabled.
    pub die_with_parent: bool,
}

/// A parsed event from the worker's stdout.
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    /// Valid event envelope parsed from JSON Lines.
    Valid(super::coding_event::EventEnvelope),
    /// Malformed line that could not be parsed.
    Malformed { raw: String },
}

/// Status of a running worker process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerStatus {
    /// Worker is running.
    Running,
    /// Worker exited with a status code.
    Exited { status: i32 },
    /// Worker was killed (by signal or timeout).
    Killed { reason: String },
}

/// Environment variable allowed in the worker.
#[derive(Debug, Clone)]
pub struct WorkerEnvVar {
    pub name: String,
    pub value: String,
}

/// Port for managing nsjail coding worker processes.
///
/// The coordinator uses this trait to launch, monitor, communicate with,
/// and tear down worker processes.
///
/// Includes `as_any_mut()` to support safe downcasting in test code
/// (e.g. to access `MockWorkerRuntime`-specific injection methods).
pub trait WorkerRuntime {
    /// Launch a new worker process inside nsjail.
    fn launch(&mut self, config: &WorkerLaunchConfig) -> Result<u32, String>;

    /// Send a JSON command to the worker via stdin.
    fn send_command(&mut self, pid: u32, command: &str) -> Result<(), String>;

    /// Read the next event from the worker's stdout (non-blocking).
    fn read_event(&mut self, pid: u32) -> Option<WorkerEvent>;

    /// Read accumulated stderr output for diagnostics.
    fn read_stderr(&mut self, pid: u32) -> String;

    /// Check the current status of the worker process.
    fn status(&self, pid: u32) -> WorkerStatus;

    /// Send SIGTERM to the worker, then SIGKILL after timeout.
    fn kill(&mut self, pid: u32) -> Result<(), String>;

    /// Check if the worker is still running.
    fn is_alive(&self, pid: u32) -> bool;

    /// Return the nsjail arguments that would be used for the given config.
    fn nsjail_args(&self, config: &WorkerLaunchConfig) -> Vec<String>;

    /// Return the environment variables the worker would receive.
    fn worker_env(&self, config: &WorkerLaunchConfig) -> Vec<WorkerEnvVar>;

    /// Clean up all resources for the given worker PID.
    fn cleanup(&mut self, pid: u32);

    /// Downcast support for test code. Returns `self` as `&mut dyn Any`.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
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
