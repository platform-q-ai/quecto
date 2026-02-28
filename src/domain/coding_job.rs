use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Job state machine for a coding job.
///
/// State transitions (from contract `state_machine.job_transitions`):
/// ```text
/// queued     -> [preparing, failed, canceled]
/// preparing  -> [running, blocked, failed, canceled]
/// running    -> [blocked, failed, succeeded, canceled]
/// blocked    -> [running, failed, canceled]
/// failed     -> []  (terminal)
/// succeeded  -> []  (terminal)
/// canceled   -> []  (terminal)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Preparing,
    Running,
    Blocked,
    Failed,
    Succeeded,
    Canceled,
}

impl JobState {
    /// Returns true if the state is terminal (no further transitions allowed).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Succeeded | Self::Canceled)
    }

    /// Returns the set of states this state can transition to.
    pub fn allowed_transitions(self) -> &'static [JobState] {
        match self {
            Self::Queued => &[Self::Preparing, Self::Failed, Self::Canceled],
            Self::Preparing => &[Self::Running, Self::Blocked, Self::Failed, Self::Canceled],
            Self::Running => &[Self::Blocked, Self::Failed, Self::Succeeded, Self::Canceled],
            Self::Blocked => &[Self::Running, Self::Failed, Self::Canceled],
            Self::Failed | Self::Succeeded | Self::Canceled => &[],
        }
    }

    /// Check whether a transition to `target` is valid.
    pub fn can_transition_to(self, target: JobState) -> bool {
        self.allowed_transitions().contains(&target)
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Succeeded => "succeeded",
            Self::Canceled => "canceled",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for JobState {
    type Err = JobStateParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Self::Queued),
            "preparing" => Ok(Self::Preparing),
            "running" => Ok(Self::Running),
            "blocked" => Ok(Self::Blocked),
            "failed" => Ok(Self::Failed),
            "succeeded" => Ok(Self::Succeeded),
            "canceled" => Ok(Self::Canceled),
            _ => Err(JobStateParseError(s.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid job state: {0}")]
pub struct JobStateParseError(String);

/// Why a job was canceled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    UserRequest,
    WallTimeout,
    ResourceLimit,
    CoordinatorPolicy,
}

impl std::fmt::Display for CancelReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::UserRequest => "user_request",
            Self::WallTimeout => "wall_timeout",
            Self::ResourceLimit => "resource_limit",
            Self::CoordinatorPolicy => "coordinator_policy",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for CancelReason {
    type Err = CancelReasonParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user_request" => Ok(Self::UserRequest),
            "wall_timeout" => Ok(Self::WallTimeout),
            "resource_limit" => Ok(Self::ResourceLimit),
            "coordinator_policy" => Ok(Self::CoordinatorPolicy),
            _ => Err(CancelReasonParseError(s.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid cancel reason: {0}")]
pub struct CancelReasonParseError(String);

/// Who initiated a cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelInitiator {
    User,
    Coordinator,
    System,
}

impl std::fmt::Display for CancelInitiator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::User => "user",
            Self::Coordinator => "coordinator",
            Self::System => "system",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for CancelInitiator {
    type Err = CancelInitiatorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "coordinator" => Ok(Self::Coordinator),
            "system" => Ok(Self::System),
            _ => Err(CancelInitiatorParseError(s.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid cancel initiator: {0}")]
pub struct CancelInitiatorParseError(String);

/// Error classification for failed jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Timeout,
    Oom,
    SeccompViolation,
    ToolError,
    LlmRefusal,
    Internal,
    CoordinatorCrash,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Timeout => "timeout",
            Self::Oom => "oom",
            Self::SeccompViolation => "seccomp_violation",
            Self::ToolError => "tool_error",
            Self::LlmRefusal => "llm_refusal",
            Self::Internal => "internal",
            Self::CoordinatorCrash => "coordinator_crash",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for ErrorCode {
    type Err = ErrorCodeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "timeout" => Ok(Self::Timeout),
            "oom" => Ok(Self::Oom),
            "seccomp_violation" => Ok(Self::SeccompViolation),
            "tool_error" => Ok(Self::ToolError),
            "llm_refusal" => Ok(Self::LlmRefusal),
            "internal" => Ok(Self::Internal),
            "coordinator_crash" => Ok(Self::CoordinatorCrash),
            _ => Err(ErrorCodeParseError(s.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid error code: {0}")]
pub struct ErrorCodeParseError(String);

/// Job priority level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Medium,
    High,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for Priority {
    type Err = PriorityParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            _ => Err(PriorityParseError(s.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid priority: {0}")]
pub struct PriorityParseError(String);

/// A coding job — a single unit of isolated coding work.
#[derive(Debug, Clone)]
pub struct CodingJob {
    /// Globally unique job identifier.
    pub job_id: String,
    /// Run identifier (run 1:N jobs; MVP: 1:1).
    pub run_id: String,
    /// Current state in the job state machine.
    pub state: JobState,
    /// The high-level goal for this coding task.
    pub goal: String,
    /// Repository identifier or local path.
    pub repo: String,
    /// Base ref (branch/tag/commit) to work from.
    pub base_ref: String,
    /// Working branch name created for this job.
    pub branch: String,
    /// Job priority.
    pub priority: Priority,
    /// Profile name (e.g. "backend", "default").
    pub profile: String,
    /// Optional user-defined labels.
    pub labels: Vec<String>,
    /// Optional skill names to apply.
    pub skills: Vec<String>,
    /// Optional wall-clock timeout in seconds.
    pub max_wall_seconds: Option<u64>,
    /// Worker PID (set when worker starts).
    pub worker_pid: Option<u32>,
    /// Error code (set when state is Failed).
    pub error_code: Option<ErrorCode>,
    /// Error detail message (set when state is Failed).
    pub error_detail: Option<String>,
    /// Whether the failure is retriable.
    pub is_retriable: Option<bool>,
    /// Cancel reason (set when state is Canceled).
    pub cancel_reason: Option<CancelReason>,
    /// Who initiated the cancellation.
    pub cancel_initiated_by: Option<CancelInitiator>,
    /// Summary text.
    pub summary: Option<String>,
    /// Artifact IDs produced by this job.
    pub artifacts: Vec<String>,
    /// Progress percentage (0-100).
    pub progress: Option<u32>,
    /// Wall-clock duration in milliseconds (set on completion).
    pub duration_ms: Option<u64>,
    /// Unix timestamp (seconds) when the job was created (queued).
    pub created_at: u64,
    /// Unix timestamp (seconds) when the job last changed state.
    pub state_entered_at: u64,
    /// ISO 8601 timestamp of the most-recent event received for this job.
    pub last_event_ts: Option<String>,
    /// Event type of the most-recent event received for this job.
    pub last_event_type: Option<String>,
}

/// Required fields to construct a new coding job.
#[derive(Debug, Clone)]
pub struct CodingJobInit {
    pub job_id: String,
    pub run_id: String,
    pub goal: String,
    pub repo: String,
    pub base_ref: String,
    pub branch: String,
}

/// Return the current Unix timestamp in whole seconds.
pub fn now_unix_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl CodingJob {
    /// Create a new job in Queued state.
    ///
    /// `now` is the Unix timestamp (seconds) for `created_at` and
    /// `state_entered_at`. Callers should use `now_unix_secs()` in
    /// production; tests can inject a deterministic value.
    pub fn new(init: CodingJobInit, now: u64) -> Self {
        Self {
            job_id: init.job_id,
            run_id: init.run_id,
            state: JobState::Queued,
            goal: init.goal,
            repo: init.repo,
            base_ref: init.base_ref,
            branch: init.branch,
            priority: Priority::default(),
            profile: "default".to_string(),
            labels: vec![],
            skills: vec![],
            max_wall_seconds: None,
            worker_pid: None,
            error_code: None,
            error_detail: None,
            is_retriable: None,
            cancel_reason: None,
            cancel_initiated_by: None,
            summary: None,
            artifacts: vec![],
            progress: None,
            duration_ms: None,
            created_at: now,
            state_entered_at: now,
            last_event_ts: None,
            last_event_type: None,
        }
    }

    /// Attempt a state transition. Returns an error if the transition is invalid.
    ///
    /// `now` is the Unix timestamp (seconds) for `state_entered_at`.
    /// Callers should use `now_unix_secs()` in production; tests can
    /// inject a deterministic value.
    pub fn transition_to(&mut self, target: JobState, now: u64) -> Result<(), TransitionError> {
        if self.state.can_transition_to(target) {
            self.state = target;
            self.state_entered_at = now;
            Ok(())
        } else {
            Err(TransitionError {
                from: self.state,
                to: target,
            })
        }
    }
}

/// Error returned when a state transition is invalid.
#[derive(Debug, Error)]
#[error("invalid transition from {from} to {to}")]
pub struct TransitionError {
    pub from: JobState,
    pub to: JobState,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed timestamp for deterministic tests.
    const T: u64 = 1_700_000_000;

    fn test_init(id: &str) -> CodingJobInit {
        CodingJobInit {
            job_id: id.into(),
            run_id: format!("r_{id}"),
            goal: "goal".into(),
            repo: "repo".into(),
            base_ref: "main".into(),
            branch: format!("quecto/job/{id}"),
        }
    }

    // --- JobState ---

    #[test]
    fn test_terminal_states() {
        assert!(JobState::Failed.is_terminal());
        assert!(JobState::Succeeded.is_terminal());
        assert!(JobState::Canceled.is_terminal());
        assert!(!JobState::Queued.is_terminal());
        assert!(!JobState::Preparing.is_terminal());
        assert!(!JobState::Running.is_terminal());
        assert!(!JobState::Blocked.is_terminal());
    }

    #[test]
    fn test_all_queued_transitions() {
        let allowed = JobState::Queued.allowed_transitions();
        assert!(allowed.contains(&JobState::Preparing));
        assert!(allowed.contains(&JobState::Failed));
        assert!(allowed.contains(&JobState::Canceled));
        assert!(!allowed.contains(&JobState::Running));
        assert!(!allowed.contains(&JobState::Blocked));
        assert!(!allowed.contains(&JobState::Succeeded));
    }

    #[test]
    fn test_all_preparing_transitions() {
        let allowed = JobState::Preparing.allowed_transitions();
        assert!(allowed.contains(&JobState::Running));
        assert!(allowed.contains(&JobState::Blocked));
        assert!(allowed.contains(&JobState::Failed));
        assert!(allowed.contains(&JobState::Canceled));
        assert!(!allowed.contains(&JobState::Queued));
        assert!(!allowed.contains(&JobState::Succeeded));
    }

    #[test]
    fn test_all_running_transitions() {
        let allowed = JobState::Running.allowed_transitions();
        assert!(allowed.contains(&JobState::Blocked));
        assert!(allowed.contains(&JobState::Failed));
        assert!(allowed.contains(&JobState::Succeeded));
        assert!(allowed.contains(&JobState::Canceled));
        assert!(!allowed.contains(&JobState::Queued));
        assert!(!allowed.contains(&JobState::Preparing));
    }

    #[test]
    fn test_all_blocked_transitions() {
        let allowed = JobState::Blocked.allowed_transitions();
        assert!(allowed.contains(&JobState::Running));
        assert!(allowed.contains(&JobState::Failed));
        assert!(allowed.contains(&JobState::Canceled));
        assert!(!allowed.contains(&JobState::Queued));
        assert!(!allowed.contains(&JobState::Preparing));
        assert!(!allowed.contains(&JobState::Succeeded));
        assert!(!allowed.contains(&JobState::Blocked));
    }

    #[test]
    fn test_terminal_states_have_no_transitions() {
        assert!(JobState::Failed.allowed_transitions().is_empty());
        assert!(JobState::Succeeded.allowed_transitions().is_empty());
        assert!(JobState::Canceled.allowed_transitions().is_empty());
    }

    #[test]
    fn test_can_transition_to_valid() {
        assert!(JobState::Queued.can_transition_to(JobState::Preparing));
        assert!(JobState::Running.can_transition_to(JobState::Succeeded));
        assert!(JobState::Blocked.can_transition_to(JobState::Running));
    }

    #[test]
    fn test_can_transition_to_invalid() {
        assert!(!JobState::Queued.can_transition_to(JobState::Running));
        assert!(!JobState::Queued.can_transition_to(JobState::Succeeded));
        assert!(!JobState::Failed.can_transition_to(JobState::Running));
        assert!(!JobState::Succeeded.can_transition_to(JobState::Failed));
    }

    #[test]
    fn test_display_round_trip_all_states() {
        for state in [
            JobState::Queued,
            JobState::Preparing,
            JobState::Running,
            JobState::Blocked,
            JobState::Failed,
            JobState::Succeeded,
            JobState::Canceled,
        ] {
            let s = state.to_string();
            let parsed: JobState = s.parse().unwrap();
            assert_eq!(state, parsed);
        }
    }

    #[test]
    fn test_parse_invalid_state() {
        assert!("bogus".parse::<JobState>().is_err());
    }

    // --- CodingJob ---

    #[test]
    fn test_new_job_is_queued() {
        let job = CodingJob::new(test_init("j1"), T);
        assert_eq!(job.state, JobState::Queued);
        assert_eq!(job.priority, Priority::Medium);
        assert_eq!(job.profile, "default");
        assert_eq!(job.created_at, T);
        assert_eq!(job.state_entered_at, T);
    }

    #[test]
    fn test_valid_transition() {
        let mut job = CodingJob::new(test_init("j1"), T);
        assert!(job.transition_to(JobState::Preparing, T + 1).is_ok());
        assert_eq!(job.state, JobState::Preparing);
        assert_eq!(job.state_entered_at, T + 1);
        assert!(job.transition_to(JobState::Running, T + 2).is_ok());
        assert_eq!(job.state, JobState::Running);
        assert!(job.transition_to(JobState::Succeeded, T + 3).is_ok());
        assert_eq!(job.state, JobState::Succeeded);
    }

    #[test]
    fn test_invalid_transition_returns_error() {
        let mut job = CodingJob::new(test_init("j1"), T);
        let err = job.transition_to(JobState::Running, T).unwrap_err();
        assert_eq!(err.from, JobState::Queued);
        assert_eq!(err.to, JobState::Running);
        // State should not have changed
        assert_eq!(job.state, JobState::Queued);
    }

    #[test]
    fn test_terminal_state_rejects_all_transitions() {
        let mut job = CodingJob::new(test_init("j1"), T);
        job.state = JobState::Failed;
        for target in [
            JobState::Queued,
            JobState::Preparing,
            JobState::Running,
            JobState::Blocked,
            JobState::Succeeded,
            JobState::Canceled,
        ] {
            assert!(job.transition_to(target, T).is_err());
        }
    }

    // --- CancelReason ---

    #[test]
    fn test_cancel_reason_display_round_trip() {
        for reason in [
            CancelReason::UserRequest,
            CancelReason::WallTimeout,
            CancelReason::ResourceLimit,
            CancelReason::CoordinatorPolicy,
        ] {
            let s = reason.to_string();
            let parsed: CancelReason = s.parse().unwrap();
            assert_eq!(reason, parsed);
        }
    }

    // --- CancelInitiator ---

    #[test]
    fn test_cancel_initiator_display_round_trip() {
        for initiator in [
            CancelInitiator::User,
            CancelInitiator::Coordinator,
            CancelInitiator::System,
        ] {
            let s = initiator.to_string();
            let parsed: CancelInitiator = s.parse().unwrap();
            assert_eq!(initiator, parsed);
        }
    }

    // --- ErrorCode ---

    #[test]
    fn test_error_code_display_round_trip() {
        for code in [
            ErrorCode::Timeout,
            ErrorCode::Oom,
            ErrorCode::SeccompViolation,
            ErrorCode::ToolError,
            ErrorCode::LlmRefusal,
            ErrorCode::Internal,
            ErrorCode::CoordinatorCrash,
        ] {
            let s = code.to_string();
            let parsed: ErrorCode = s.parse().unwrap();
            assert_eq!(code, parsed);
        }
    }

    #[test]
    fn test_parse_invalid_error_code() {
        assert!("bogus".parse::<ErrorCode>().is_err());
    }

    // --- Priority ---

    #[test]
    fn test_priority_default_is_medium() {
        assert_eq!(Priority::default(), Priority::Medium);
    }

    #[test]
    fn test_priority_display_round_trip() {
        for p in [Priority::Low, Priority::Medium, Priority::High] {
            let s = p.to_string();
            let parsed: Priority = s.parse().unwrap();
            assert_eq!(p, parsed);
        }
    }

    // --- Full lifecycle transition paths ---

    #[test]
    fn test_happy_path_queued_to_succeeded() {
        let mut job = CodingJob::new(test_init("j1"), T);
        assert!(job.transition_to(JobState::Preparing, T + 1).is_ok());
        assert!(job.transition_to(JobState::Running, T + 2).is_ok());
        assert!(job.transition_to(JobState::Succeeded, T + 3).is_ok());
        assert!(job.state.is_terminal());
    }

    #[test]
    fn test_blocked_and_resume_path() {
        let mut job = CodingJob::new(test_init("j1"), T);
        assert!(job.transition_to(JobState::Preparing, T + 1).is_ok());
        assert!(job.transition_to(JobState::Running, T + 2).is_ok());
        assert!(job.transition_to(JobState::Blocked, T + 3).is_ok());
        assert!(job.transition_to(JobState::Running, T + 4).is_ok());
        assert!(job.transition_to(JobState::Succeeded, T + 5).is_ok());
    }

    #[test]
    fn test_queued_direct_to_failed() {
        let mut job = CodingJob::new(test_init("j1"), T);
        assert!(job.transition_to(JobState::Failed, T + 1).is_ok());
        assert!(job.state.is_terminal());
    }

    #[test]
    fn test_queued_direct_to_canceled() {
        let mut job = CodingJob::new(test_init("j1"), T);
        assert!(job.transition_to(JobState::Canceled, T + 1).is_ok());
        assert!(job.state.is_terminal());
    }

    #[test]
    fn test_preparing_to_blocked_to_failed() {
        let mut job = CodingJob::new(test_init("j1"), T);
        assert!(job.transition_to(JobState::Preparing, T + 1).is_ok());
        assert!(job.transition_to(JobState::Blocked, T + 2).is_ok());
        assert!(job.transition_to(JobState::Failed, T + 3).is_ok());
    }
}
