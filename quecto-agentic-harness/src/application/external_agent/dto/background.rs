//! Bash commands the external agent tracks as tasks (#2285).

/// The projection tracks at most this many jobs. Past it a finished job
/// is dropped first (the oldest), and a running one only when none has
/// finished.
pub const BACKGROUND_JOB_CAPACITY: usize = 64;

/// A Bash command the CLI tracks as a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundJob {
    pub task_id: String,
    pub tool_use_id: Option<String>,
    pub description: Option<String>,
    pub is_backgrounded: bool,
    /// The last status a notification gave; `None` while it runs.
    pub status: Option<String>,
}

/// The task statuses that end a job.
pub const TERMINAL_TASK_STATUSES: &[&str] = &["completed", "failed", "killed", "stopped"];

impl BackgroundJob {
    /// Whether a notification reported a terminal status.
    pub fn is_finished(&self) -> bool {
        self.status
            .as_deref()
            .is_some_and(|status| TERMINAL_TASK_STATUSES.contains(&status))
    }
}
