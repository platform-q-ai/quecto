use crate::protocol::session_payloads;

pub(super) fn session_stats_text(stats: &session_payloads::SessionStats) -> String {
    let mut text = format!(
        "Session: {} | Messages: {} | Tokens: ↑{} ↓{}",
        stats.session_key, stats.total_messages, stats.input_tokens, stats.output_tokens
    );
    if stats.cache_read_tokens > 0 || stats.cache_write_tokens > 0 {
        text.push_str(&format!(
            " | Cache: read {} write {}",
            stats.cache_read_tokens, stats.cache_write_tokens
        ));
    }
    if let Some(ratio) = stats.cache_hit_ratio {
        text.push_str(&format!(" | Cache hit: {:.1}%", ratio * 100.0));
    }
    // #1855: usage the harness could not price is unknown, never $0.
    let cost = stats.cost_micro_usd as f64 / 1_000_000.0;
    match (stats.cost_micro_usd > 0, stats.unpriced_attempts > 0) {
        (true, false) => text.push_str(&format!(" | Cost: ${cost:.6}")),
        (true, true) => text.push_str(&format!(" | Cost: ${cost:.6} + unpriced")),
        (false, true) => text.push_str(" | Cost: unpriced"),
        (false, false) => {}
    }
    if let Some((used, max)) = stats.context_usage {
        text.push_str(&format!(" | Context: {used}/{max}"));
    }
    text
}

#[cfg(test)]
#[path = "app_session_stats_text_tests.rs"]
mod tests;
