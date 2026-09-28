//! Bash commands the external agent tracks as tasks (#2285).

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
