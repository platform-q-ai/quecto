use serde::{Deserialize, Serialize};

use super::coding_job::{CancelReason, ErrorCode, JobState, Priority};

// ============================================================================
// Run command
// ============================================================================

/// Request to start a new coding job.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub run_id: String,
    pub job_id: String,
    pub state: JobState,
}

// ============================================================================
// Status command
// ============================================================================

/// Request to query job or run status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// Response from a `status` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub job_id: String,
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
pub struct CancelRequest {
    pub job_id: String,
}

/// Response from a `cancel` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelResponse {
    pub job_id: String,
    pub state: JobState,
}

// ============================================================================
// Cleanup command
// ============================================================================

/// Request to clean up a job's filesystem artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupRequest {
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
    pub job_id: String,
    pub cleaned: bool,
}

// ============================================================================
// List command
// ============================================================================

/// Request to list jobs, optionally filtered by state.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub job_id: String,
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
}
