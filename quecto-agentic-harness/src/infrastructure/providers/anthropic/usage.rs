use crate::domain::message::UsageInfo;

pub(super) fn context_input_tokens(
    prompt_tokens: u32,
    cache_read_tokens: Option<u32>,
    cache_write_tokens: Option<u32>,
) -> u32 {
    prompt_tokens
        .saturating_add(cache_read_tokens.unwrap_or(0))
        .saturating_add(cache_write_tokens.unwrap_or(0))
}

pub(super) fn parse_usage(u: &serde_json::Map<String, serde_json::Value>) -> UsageInfo {
    let prompt_tokens = u["input_tokens"].as_u64().unwrap_or(0) as u32;
    let cache_read_tokens = u
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let cache_write_tokens = u
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    UsageInfo {
        prompt_tokens,
        completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
        cache_read_tokens,
        cache_write_tokens,
        // Anthropic reports `input_tokens` as the non-cached delta only;
        // true context occupancy adds the cached read + creation tokens.
        context_tokens: Some(context_input_tokens(
            prompt_tokens,
            cache_read_tokens,
            cache_write_tokens,
        )),
        cost: None,
    }
}
