//! Live status vocabulary for spawned subagents, split from
//! `subagent_registry.rs` to keep that file within the size baseline.

use std::fmt;

/// Live status of a spawned subagent, updated by the monitor task (#522).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SubagentStatus {
    /// Child process spawned but not yet confirmed running.
    #[default]
    Starting,
    /// Agent finished processing and is waiting for the next prompt.
    Idle,
    /// Agent is actively processing a prompt or executing a tool.
    Running,
    /// Last tool execution returned an error.
    Error,
    /// Child process exited (connection closed or process reaped).
    Exited,
}

impl SubagentStatus {
    /// True while this status represents active work that should keep ancestors
    /// effectively running.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }

    /// Wire-format string for the UDS protocol (lowercase, zero-alloc).
    pub fn to_wire_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Error => "error",
            Self::Exited => "exited",
        }
    }

    /// Parse a wire-format status string back into a [`SubagentStatus`].
    /// Unknown values map to `Starting` (the conservative default). Inverse of
    /// [`to_wire_str`](Self::to_wire_str); used when merging a descendant's
    /// forwarded state into the registry (#815).
    pub fn from_wire_str(s: &str) -> Self {
        match s {
            "idle" => Self::Idle,
            "running" => Self::Running,
            "error" => Self::Error,
            "exited" => Self::Exited,
            _ => Self::Starting,
        }
    }
}

impl fmt::Display for SubagentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Starting => write!(f, "Starting"),
            Self::Idle => write!(f, "Idle"),
            Self::Running => write!(f, "Running"),
            Self::Error => write!(f, "Error"),
            Self::Exited => write!(f, "Exited"),
        }
    }
}
