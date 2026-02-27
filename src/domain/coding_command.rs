use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use super::coding_contract::is_valid_runtime_id;
use super::coding_job::{CancelReason, ErrorCode, JobState, Priority};

fn deserialize_runtime_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if !is_valid_runtime_id(&value) {
        return Err(D::Error::custom("invalid runtime id"));
    }
    Ok(value)
}

fn deserialize_optional_runtime_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    match value {
        Some(id) if is_valid_runtime_id(&id) => Ok(Some(id)),
        Some(_) => Err(D::Error::custom("invalid runtime id")),
        None => Ok(None),
    }
}

// ============================================================================
// Create command
// ============================================================================

/// Request to create a new repository in the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    /// Repository name (e.g. "my-project"). Will be created under workspace.
    pub name: String,
    /// Optional description for the initial commit message.
    #[serde(default)]
    pub description: Option<String>,
}

/// Response from a `create` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResponse {
    pub name: String,
    pub path: String,
    pub created: bool,
}

// ============================================================================
// Import command
// ============================================================================

/// Request to clone a remote repository into the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    /// Remote URL (HTTPS or SSH) to clone from.
    ///
    /// URL safety validation (scheme allowlist, SSRF prevention) is deferred
    /// to the infrastructure layer (`is_safe_import_url`) rather than the
    /// domain, because the set of safe schemes is an infrastructure concern.
    pub url: String,
    /// Optional local name. Defaults to the repo name derived from the URL.
    #[serde(default)]
    pub name: Option<String>,
}

/// Response from an `import` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResponse {
    pub name: String,
    pub path: String,
    pub imported: bool,
}

// ============================================================================
// Run command
// ============================================================================

/// Request to start a new coding job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub goal: String,
    pub repo: String,
    pub base_ref: String,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_wall_seconds: Option<u64>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

fn default_profile() -> String {
    "default".to_string()
}

/// Response from a successful `run` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResponse {
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub run_id: String,
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub job_id: String,
    pub state: JobState,
}

// ============================================================================
// Status command
// ============================================================================

/// Request to query job or run status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "RawStatusRequest")]
pub struct StatusRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStatusRequest {
    #[serde(default, deserialize_with = "deserialize_optional_runtime_id")]
    job_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_runtime_id")]
    run_id: Option<String>,
}

impl TryFrom<RawStatusRequest> for StatusRequest {
    type Error = String;

    fn try_from(value: RawStatusRequest) -> Result<Self, Self::Error> {
        match (&value.job_id, &value.run_id) {
            (Some(_), Some(_)) => Err("exactly one of job_id or run_id is required".to_string()),
            (None, None) => Err("exactly one of job_id or run_id is required".to_string()),
            _ => Ok(Self {
                job_id: value.job_id,
                run_id: value.run_id,
            }),
        }
    }
}

/// Response from a `status` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub job_id: String,
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub run_id: String,
    pub state: JobState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u32>,
    #[serde(default)]
    pub todos: Vec<TodoItem>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<CancelReason>,
    /// Unix timestamp (seconds) when the job entered the current state.
    /// Allows callers to detect stuck/hung jobs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_entered_at: Option<u64>,
    /// Unix timestamp (seconds) when the job was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// ISO 8601 timestamp of the most-recent event for this job.
    /// `None` means no events have been emitted yet (job is queued/preparing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_ts: Option<String>,
    /// Type string of the most-recent event (e.g. `"tool.result"`, `"job.status"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_type: Option<String>,
}

/// A todo item as returned in status responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub todo_id: String,
    pub title: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
}

// ============================================================================
// Cancel command
// ============================================================================

/// Request to cancel a job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub job_id: String,
}

/// Response from a `cancel` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelResponse {
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub job_id: String,
    pub state: JobState,
}

// ============================================================================
// Cleanup command
// ============================================================================

/// Request to clean up a job's filesystem artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRequest {
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub job_id: String,
    #[serde(default = "default_keep_artifacts")]
    pub keep_artifacts: bool,
}

fn default_keep_artifacts() -> bool {
    true
}

/// Response from a `cleanup` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupResponse {
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub job_id: String,
    pub cleaned: bool,
}

// ============================================================================
// Cleanup-all command
// ============================================================================

/// Request to clean up multiple jobs in bulk.
///
/// When `state_filter` is set only jobs in those states are eligible.
/// All filtered jobs must be in terminal state or the request returns
/// `job_not_terminal` for the first non-terminal job encountered.
/// Setting `terminal_only: true` (the default) silently skips any
/// non-terminal jobs rather than erroring.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupAllRequest {
    /// If set, only clean up jobs in these states.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_filter: Option<Vec<JobState>>,
    /// Keep job artifacts on disk (default: true).
    #[serde(default = "default_keep_artifacts")]
    pub keep_artifacts: bool,
    /// Skip non-terminal jobs instead of returning an error (default: true).
    #[serde(default = "default_true")]
    pub terminal_only: bool,
}

fn default_true() -> bool {
    true
}

/// Response from a `cleanup_all` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupAllResponse {
    /// How many jobs were cleaned up.
    pub cleaned_count: usize,
    /// Job IDs that were cleaned.
    pub cleaned_job_ids: Vec<String>,
    /// Job IDs that were skipped (non-terminal when `terminal_only: true`).
    pub skipped_job_ids: Vec<String>,
}

// ============================================================================
// List command
// ============================================================================

/// Request to list jobs, optionally filtered by state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_filter: Option<Vec<JobState>>,
}

/// Response from a `list` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    pub jobs: Vec<ListJobEntry>,
}

/// A single job entry in a list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListJobEntry {
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub job_id: String,
    #[serde(deserialize_with = "deserialize_runtime_id")]
    pub run_id: String,
    pub state: JobState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Unix timestamp (seconds) when the job was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// Unix timestamp (seconds) when the job entered its current state.
    /// The difference `now - state_entered_at` is the time spent in the
    /// current state — useful for detecting stuck `running` jobs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_entered_at: Option<u64>,
    /// ISO 8601 timestamp of the most-recent event for this job.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_ts: Option<String>,
    /// Type of the most-recent event (e.g. `"tool.result"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_type: Option<String>,
}

// ============================================================================
// Command errors
// ============================================================================

/// Error codes returned by the command API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandError {
    /// Repo identifier could not be resolved.
    InvalidRepo,
    /// Base ref does not exist in repo.
    /// The `detail` field (when present) includes the repo's default branch
    /// and a list of available local refs, formatted as:
    /// `"default_branch=<b>; available_refs=[<r1>, <r2>, ...]"`
    /// This helps callers recover without a separate API call.
    InvalidBaseRef,
    /// Base ref does not exist, with enriched detail for caller recovery.
    /// Distinct from `InvalidBaseRef` so existing match arms keep working.
    InvalidBaseRefDetail(String),
    /// Job rejected by policy.
    PolicyDenied,
    /// Requested skill not found on disk.
    SkillNotFound,
    /// No job or run exists with the given ID.
    NotFound,
    /// Job is not in a terminal state (for cleanup).
    JobNotTerminal,
    /// Job cannot transition from its current state.
    InvalidTransition,
    /// Repository name is invalid (bad characters, traversal, etc.).
    InvalidName,
    /// Repository already exists in the workspace.
    AlreadyExists,
    /// Remote URL is invalid or disallowed.
    InvalidUrl,
    /// Git operation failed (clone, init, etc.).
    GitFailed(String),
    /// Internal error (e.g. poisoned lock, unexpected state).
    Internal(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::InvalidRepo => "invalid_repo",
            Self::InvalidBaseRef => "invalid_base_ref",
            Self::InvalidBaseRefDetail(detail) => {
                return write!(f, "invalid_base_ref: {detail}");
            }
            Self::PolicyDenied => "policy_denied",
            Self::SkillNotFound => "skill_not_found",
            Self::NotFound => "not_found",
            Self::JobNotTerminal => "job_not_terminal",
            Self::InvalidTransition => "invalid_transition",
            Self::InvalidName => "invalid_name",
            Self::AlreadyExists => "already_exists",
            Self::InvalidUrl => "invalid_url",
            Self::GitFailed(msg) => return write!(f, "git_failed: {msg}"),
            Self::Internal(msg) => return write!(f, "internal: {msg}"),
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for CommandError {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "invalid_repo" => Ok(Self::InvalidRepo),
            "invalid_base_ref" => Ok(Self::InvalidBaseRef),
            "policy_denied" => Ok(Self::PolicyDenied),
            "skill_not_found" => Ok(Self::SkillNotFound),
            "not_found" => Ok(Self::NotFound),
            "job_not_terminal" => Ok(Self::JobNotTerminal),
            "invalid_transition" => Ok(Self::InvalidTransition),
            "invalid_name" => Ok(Self::InvalidName),
            "already_exists" => Ok(Self::AlreadyExists),
            "invalid_url" => Ok(Self::InvalidUrl),
            s if s.starts_with("invalid_base_ref: ") => Ok(Self::InvalidBaseRefDetail(
                s["invalid_base_ref: ".len()..].to_string(),
            )),
            s if s.starts_with("git_failed: ") => {
                Ok(Self::GitFailed(s["git_failed: ".len()..].to_string()))
            }
            s if s.starts_with("internal: ") => {
                Ok(Self::Internal(s["internal: ".len()..].to_string()))
            }
            _ => Err(format!("unknown command error: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_error_display_round_trip() {
        for err in [
            CommandError::InvalidRepo,
            CommandError::InvalidBaseRef,
            CommandError::PolicyDenied,
            CommandError::SkillNotFound,
            CommandError::NotFound,
            CommandError::JobNotTerminal,
            CommandError::InvalidTransition,
            CommandError::InvalidName,
            CommandError::AlreadyExists,
            CommandError::InvalidUrl,
        ] {
            let s = err.to_string();
            let parsed: CommandError = s.parse().unwrap();
            assert_eq!(err, parsed);
        }
    }

    #[test]
    fn test_git_failed_display_round_trip() {
        let err = CommandError::GitFailed("clone failed".to_string());
        let s = err.to_string();
        let parsed: CommandError = s.parse().unwrap();
        assert_eq!(err, parsed);
    }

    #[test]
    fn test_create_request_basic() {
        let json = r#"{"name":"my-project"}"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "my-project");
        assert!(req.description.is_none());
    }

    #[test]
    fn test_create_request_with_description() {
        let json = r#"{"name":"my-project","description":"A new thing"}"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.description.as_deref(), Some("A new thing"));
    }

    #[test]
    fn test_import_request_basic() {
        let json = r#"{"url":"https://github.com/org/repo.git"}"#;
        let req: ImportRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.url, "https://github.com/org/repo.git");
        assert!(req.name.is_none());
    }

    #[test]
    fn test_import_request_with_name() {
        let json = r#"{"url":"https://github.com/org/repo.git","name":"my-repo"}"#;
        let req: ImportRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name.as_deref(), Some("my-repo"));
    }

    #[test]
    fn test_run_request_default_priority() {
        let json = r#"{"goal":"test","repo":"r","base_ref":"main"}"#;
        let req: RunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.priority, Priority::Medium);
        assert_eq!(req.profile, "default");
    }

    #[test]
    fn test_cleanup_request_default_keep_artifacts() {
        let json = r#"{"job_id":"j1"}"#;
        let req: CleanupRequest = serde_json::from_str(json).unwrap();
        assert!(req.keep_artifacts);
    }

    #[test]
    fn test_list_request_no_filter() {
        let json = r#"{}"#;
        let req: ListRequest = serde_json::from_str(json).unwrap();
        assert!(req.state_filter.is_none());
    }

    #[test]
    fn test_list_request_with_filter() {
        let json = r#"{"state_filter":["running","failed"]}"#;
        let req: ListRequest = serde_json::from_str(json).unwrap();
        let filter = req.state_filter.unwrap();
        assert_eq!(filter.len(), 2);
        assert_eq!(filter[0], JobState::Running);
        assert_eq!(filter[1], JobState::Failed);
    }

    #[test]
    fn test_cancel_request_rejects_invalid_job_id() {
        let json = r#"{"job_id":"job/1"}"#;
        let err = serde_json::from_str::<CancelRequest>(json).unwrap_err();
        assert!(err.to_string().contains("invalid runtime id"));
    }

    #[test]
    fn test_status_request_rejects_invalid_run_id() {
        let json = r#"{"run_id":"run.1"}"#;
        let err = serde_json::from_str::<StatusRequest>(json).unwrap_err();
        assert!(err.to_string().contains("invalid runtime id"));
    }

    #[test]
    fn test_status_request_rejects_missing_both_ids() {
        let json = r#"{}"#;
        let err = serde_json::from_str::<StatusRequest>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("exactly one of job_id or run_id is required")
        );
    }

    #[test]
    fn test_status_request_rejects_when_both_ids_present() {
        let json = r#"{"job_id":"job_1","run_id":"run_1"}"#;
        let err = serde_json::from_str::<StatusRequest>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("exactly one of job_id or run_id is required")
        );
    }

    #[test]
    fn test_status_request_rejects_unknown_field() {
        let json = r#"{"job_id":"job_1","extra":true}"#;
        let err = serde_json::from_str::<StatusRequest>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn test_run_request_rejects_unknown_field() {
        let json = r#"{"goal":"g","repo":"r","base_ref":"main","extra":"x"}"#;
        let err = serde_json::from_str::<RunRequest>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn test_cancel_request_rejects_unknown_field() {
        let json = r#"{"job_id":"job_1","extra":true}"#;
        let err = serde_json::from_str::<CancelRequest>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn test_cleanup_request_rejects_unknown_field() {
        let json = r#"{"job_id":"job_1","extra":true}"#;
        let err = serde_json::from_str::<CleanupRequest>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn test_list_request_rejects_unknown_field() {
        let json = r#"{"state_filter":["running"],"extra":true}"#;
        let err = serde_json::from_str::<ListRequest>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }
}
