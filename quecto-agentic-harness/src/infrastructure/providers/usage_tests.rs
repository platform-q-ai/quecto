use super::*;

#[test]
fn openai_reads_prompt_completion_and_prompt_context() {
    let v = serde_json::json!({
        "prompt_tokens": 12, "completion_tokens": 7, "total_tokens": 19
    });
    let u = parse_openai_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 12);
    assert_eq!(u.completion_tokens, 7);
    assert_eq!(u.context_tokens, Some(12));
    assert_eq!(u.cache_read_tokens, None);
}

#[test]
fn openai_absent_cached_tokens_remains_unreported_without_losing_base_usage() {
    let v = serde_json::json!({ "prompt_tokens": 3, "completion_tokens": 4 });
    let u = parse_openai_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 3);
    assert_eq!(u.completion_tokens, 4);
    assert_eq!(u.cache_read_tokens, None);
    assert_eq!(u.context_tokens, Some(3));
}

#[test]
fn codex_reads_input_output_and_cached() {
    let v = serde_json::json!({
        "input_tokens": 100, "output_tokens": 40,
        "input_tokens_details": { "cached_tokens": 30 }
    });
    let u = parse_codex_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 70);
    assert_eq!(u.completion_tokens, 40);
    assert_eq!(u.cache_read_tokens, Some(30));
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn openai_normalizes_cached_tokens_as_subset_of_prompt() {
    let v = serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "total_tokens": 120,
        "prompt_tokens_details": { "cached_tokens": 30 }
    });

    let u = parse_openai_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 70);
    assert_eq!(u.completion_tokens, 20);
    assert_eq!(u.cache_read_tokens, Some(30));
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn openai_preserves_reported_zero_cached_tokens() {
    let v = serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "prompt_tokens_details": { "cached_tokens": 0 }
    });

    let u = parse_openai_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.cache_read_tokens, Some(0));
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn openai_ignores_malformed_cached_tokens_without_losing_base_usage() {
    let v = serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "prompt_tokens_details": { "cached_tokens": "oops" }
    });

    let u = parse_openai_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.completion_tokens, 20);
    assert_eq!(u.cache_read_tokens, None);
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn openai_ignores_overflow_cached_tokens_without_losing_base_usage() {
    let v = serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "prompt_tokens_details": { "cached_tokens": 4294967296_u64 }
    });

    let u = parse_openai_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.completion_tokens, 20);
    assert_eq!(u.cache_read_tokens, None);
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn openai_cached_tokens_saturate_when_larger_than_prompt() {
    let v = serde_json::json!({
        "prompt_tokens": 10,
        "completion_tokens": 20,
        "prompt_tokens_details": { "cached_tokens": 30 }
    });

    let u = parse_openai_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 0);
    assert_eq!(u.cache_read_tokens, Some(10));
    assert_eq!(u.context_tokens, Some(10));
}

#[test]
fn codex_normalizes_cached_tokens_as_subset_of_input() {
    let v = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 20,
        "input_tokens_details": { "cached_tokens": 30 }
    });

    let u = parse_codex_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 70);
    assert_eq!(u.completion_tokens, 20);
    assert_eq!(u.cache_read_tokens, Some(30));
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn openai_and_codex_cached_subset_payloads_normalize_equivalently() {
    let openai = serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "prompt_tokens_details": { "cached_tokens": 30 }
    });
    let codex = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 20,
        "input_tokens_details": { "cached_tokens": 30 }
    });

    let openai_usage = parse_openai_usage(openai.as_object().unwrap());
    let codex_usage = parse_codex_usage(codex.as_object().unwrap());

    assert_eq!(openai_usage.prompt_tokens, codex_usage.prompt_tokens);
    assert_eq!(
        openai_usage.completion_tokens,
        codex_usage.completion_tokens
    );
    assert_eq!(
        openai_usage.cache_read_tokens,
        codex_usage.cache_read_tokens
    );
    assert_eq!(
        openai_usage.cache_write_tokens,
        codex_usage.cache_write_tokens
    );
    assert_eq!(openai_usage.context_tokens, codex_usage.context_tokens);
}

#[test]
fn codex_preserves_reported_zero_cached_tokens() {
    let v = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 20,
        "input_tokens_details": { "cached_tokens": 0 }
    });

    let u = parse_codex_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.cache_read_tokens, Some(0));
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn codex_absent_cached_tokens_remains_unreported_without_losing_base_usage() {
    let v = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 20
    });

    let u = parse_codex_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.completion_tokens, 20);
    assert_eq!(u.cache_read_tokens, None);
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn codex_ignores_malformed_cached_tokens_without_losing_base_usage() {
    let v = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 20,
        "input_tokens_details": { "cached_tokens": "oops" }
    });

    let u = parse_codex_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.completion_tokens, 20);
    assert_eq!(u.cache_read_tokens, None);
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn codex_ignores_overflow_cached_tokens_without_losing_base_usage() {
    let v = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 20,
        "input_tokens_details": { "cached_tokens": 4294967296_u64 }
    });

    let u = parse_codex_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.completion_tokens, 20);
    assert_eq!(u.cache_read_tokens, None);
    assert_eq!(u.context_tokens, Some(100));
}

#[test]
fn codex_cached_tokens_saturate_when_larger_than_input() {
    let v = serde_json::json!({
        "input_tokens": 10,
        "output_tokens": 20,
        "input_tokens_details": { "cached_tokens": 30 }
    });

    let u = parse_codex_usage(v.as_object().unwrap());

    assert_eq!(u.prompt_tokens, 0);
    assert_eq!(u.cache_read_tokens, Some(10));
    assert_eq!(u.context_tokens, Some(10));
}
