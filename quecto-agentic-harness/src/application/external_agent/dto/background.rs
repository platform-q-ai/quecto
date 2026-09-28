//! Bash commands the external agent tracks as tasks (#2285).

/// The projection tracks at most this many live jobs; the oldest is
/// dropped past it.
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
