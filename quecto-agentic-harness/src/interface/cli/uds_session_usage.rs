use super::*;

/// Compute session statistics from the current message history.
pub fn compute_session_stats(session_key: &str, messages: &[Message]) -> SessionStats {
    compute_session_stats_with_usage(session_key, messages, SessionUsage::default(), 0, 0)
}
/// Compute session statistics with cumulative provider usage collected by the UDS session.
/// `max_context_tokens` is the active model's context-window ceiling (0 = unknown).
pub fn compute_session_stats_with_usage(
    session_key: &str,
    messages: &[Message],
    usage: SessionUsage,
    context_tokens: usize,
    max_context_tokens: usize,
) -> SessionStats {
    let mut user_messages = 0usize;
    let mut assistant_messages = 0usize;
    let mut tool_calls_count = 0usize;
    let mut tool_results_count = 0usize;
    for msg in messages {
        match msg.role {
            Role::User => user_messages += 1,
            Role::Assistant => {
                assistant_messages += 1;
                tool_calls_count += msg.tool_calls.len();
            }
            Role::Tool => tool_results_count += 1,
            Role::System => {}
        }
    }
    SessionStats {
        runtime: Some(crate::infrastructure::runtime_identity::current()),
        request_diagnostics: usage.request_diagnostics.clone(),
        session_key: session_key.to_owned(),
        user_messages,
        assistant_messages,
        tool_calls: tool_calls_count,
        tool_results: tool_results_count,
        total_messages: messages.len(),
        cost: usage.cost_usd(),
        cost_micro_usd: usage.cost_micro_usd,
        cache_hit_ratio: usage.cache_hit_ratio(),
        tokens: usage.tokens,
        context_tokens,
        max_context_tokens,
    }
}
