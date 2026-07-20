use super::*;

#[test]
fn openai_reads_prompt_completion_and_total() {
    let v = serde_json::json!({
        "prompt_tokens": 12, "completion_tokens": 7, "total_tokens": 19
    });
    let u = parse_openai_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 12);
    assert_eq!(u.completion_tokens, 7);
    assert_eq!(u.context_tokens, Some(19));
    assert_eq!(u.cache_read_tokens, None);
}

#[test]
fn openai_missing_total_leaves_context_none() {
    let v = serde_json::json!({ "prompt_tokens": 3, "completion_tokens": 4 });
    let u = parse_openai_usage(v.as_object().unwrap());
    assert_eq!(u.context_tokens, None);
}

#[test]
fn codex_reads_input_output_and_cached() {
    let v = serde_json::json!({
        "input_tokens": 100, "output_tokens": 40,
        "input_tokens_details": { "cached_tokens": 30 }
    });
    let u = parse_codex_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.completion_tokens, 40);
    assert_eq!(u.cache_read_tokens, Some(30));
    assert_eq!(u.context_tokens, None);
}
