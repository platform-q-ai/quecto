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
    // #1855: usage the harness could not price is unknown, never $0; a
    // known $0 (a declared-free model) beside unpriced usage is shown.
    match (stats.known_cost_micro_usd, stats.unpriced_attempts > 0) {
        (Some(micro), true) => text.push_str(&format!(" | Cost: ${} + unpriced", usd(micro))),
        (Some(micro), false) if micro > 0 => text.push_str(&format!(" | Cost: ${}", usd(micro))),
        (None, true) => text.push_str(" | Cost: unpriced"),
        (Some(_), false) | (None, false) => {}
    }
    if let Some((used, max)) = stats.context_usage {
        text.push_str(&format!(" | Context: {used}/{max}"));
    }
    text
}

/// Micro-USD as dollars to six places.
fn usd(micro: u64) -> String {
    format!("{:.6}", micro as f64 / 1_000_000.0)
}

#[cfg(test)]
#[path = "app_session_stats_text_tests.rs"]
mod tests;
