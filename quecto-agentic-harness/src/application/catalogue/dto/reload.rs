//! Outcome of reloading the runtime configuration (#1849): the provider
//! runtime and the persisted tool policy re-read from the run's
//! configuration files without restarting the session.

/// What one reload attempt did to the running session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// The provider was rebuilt and swapped in, and the persisted tool
    /// policy re-applied; `unknown_policy_tools` are the persisted entries
    /// that matched no registered tool.
    Reloaded { unknown_policy_tools: Vec<String> },
    /// Nothing was swapped: either no source changed, or (on a poll) the
    /// rebuild failed and the last-good runtime was retained.
    Unchanged,
    /// The run has no reloadable configuration source.
    NotConfigured,
    /// A forced rebuild failed; the last-good runtime was retained and the
    /// error is reported to the requester.
    Failed(String),
}
