//! The wire side of a child's own session stats (#805 footer parity): the
//! request the TUI sends a child over its direct feed, and which child
//! stream events make that child's stats stale or answer the request.
//!
//! Kept in the protocol layer so feature code asks "is this child's stats
//! stale?" without matching wire DTOs itself.

use super::client::{Command, Event};

/// Request id of a child's own `get_session_stats`. The reply is applied
/// whatever its id (any stats reply on the child's feed is the child's own).
/// The id must stay: an id-less reply is what a busy child's connect-time
/// snapshot looks like, and a parent's `subagent_transport` accepts id-less
/// replies as snapshot answers to its own requests; the broadcast reply to
/// this request carries an id so it is never mistaken for one.
pub(crate) const SUBAGENT_STATS_ID: &str = "subagent-stats";

/// The request for a child's own session stats.
pub(crate) fn subagent_stats_request() -> Command {
    Command::GetSessionStats {
        id: Some(SUBAGENT_STATS_ID.to_string()),
    }
}

/// What one child stream event means for that child's session stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStatsSignal {
    /// The child's run ended (completed, failed or aborted): the harness has
    /// recorded its usage, so the footer's stats are stale.
    Stale,
    /// A `get_session_stats` reply, successful or not: a request is answered.
    Answered,
    /// Nothing to do with the child's stats.
    Unrelated,
}

/// The commands whose reply on the child's own feed means its run has
/// ended. A completed run reports `turn_end`; a failed one only an
/// `agent_error` reply; a plain abort the broadcast `abort` acknowledgement,
/// dispatched after the cancelled prompt settles. A parent's or launcher's
/// abort (`ack: accept`) is acknowledged to that client alone, so the
/// roster's running -> idle covers it instead. An allowlist: any other
/// reply leaves the stats as they are.
const RUN_END_REPLIES: &[&str] = &["agent_error", "abort"];

/// Classify one child stream event for that child's session stats.
pub(crate) fn session_stats_signal(ev: &Event) -> SessionStatsSignal {
    match ev {
        Event::TurnEnd { .. } => SessionStatsSignal::Stale,
        Event::Response { command, .. } if command == "get_session_stats" => {
            SessionStatsSignal::Answered
        }
        Event::Response { command, .. } if RUN_END_REPLIES.contains(&command.as_str()) => {
            SessionStatsSignal::Stale
        }
        _ => SessionStatsSignal::Unrelated,
    }
}

#[cfg(test)]
#[path = "subagent_stats_tests.rs"]
mod tests;
