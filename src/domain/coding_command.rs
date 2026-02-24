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
    InvalidBaseRef,
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
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::InvalidRepo => "invalid_repo",
            Self::InvalidBaseRef => "invalid_base_ref",
            Self::PolicyDenied => "policy_denied",
            Self::SkillNotFound => "skill_not_found",
            Self::NotFound => "not_found",
            Self::JobNotTerminal => "job_not_terminal",
            Self::InvalidTransition => "invalid_transition",
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
        ] {
            let s = err.to_string();
            let parsed: CommandError = s.parse().unwrap();
            assert_eq!(err, parsed);
        }
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
