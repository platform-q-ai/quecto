//! The `get_session_stats` presenter (#1855): the inspect-session-usage use
//! case's report in the stats wire shape, with the process's runtime
//! identity. Child module of `uds_session`; it computes nothing.
use super::*;
use crate::application::agent_turn::dto::session_usage::SessionUsageReport;

/// The stats wire shape of a session usage `report`.
pub fn present_session_stats(report: SessionUsageReport) -> SessionStats {
    let SessionUsageReport {
        session_key,
        messages,
        usage,
        cost_micro_usd,
        cost_usd,
        cache_hit_ratio,
        context_tokens,
        max_context_tokens,
    } = report;
    SessionStats {
        runtime: Some(crate::infrastructure::processes::runtime_identity::current()),
        request_diagnostics: usage.request_diagnostics,
        session_key,
        user_messages: messages.user,
        assistant_messages: messages.assistant,
        tool_calls: messages.tool_calls,
        tool_results: messages.tool_results,
        total_messages: messages.total,
        tokens: TokenStats {
            input: usage.tokens.input,
            output: usage.tokens.output,
            cache_read: usage.tokens.cache_read,
            cache_write: usage.tokens.cache_write,
            total: usage.tokens.total,
        },
        cost: cost_usd,
        cost_micro_usd,
        unpriced_attempts: usage.unpriced_attempts,
        cache_hit_ratio,
        context_tokens,
        max_context_tokens,
    }
}

/// The stats of a session that has used nothing yet: its history only.
pub fn history_session_stats(session_key: &str, messages: &[Message]) -> SessionStats {
    present_session_stats(InspectSessionUsage::history_report(session_key, messages))
}
