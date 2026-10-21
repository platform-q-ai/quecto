//! Behaviour-preserving efficiency cleanups for issue #996.
//!
//! These tests drive the consolidated usage parser. The item 2 preview and
//! truncation guards and the item 10 `ProviderEntry` test live beside their
//! modules (#2679).

#![cfg(test)]

// ---------------------------------------------------------------------------
// Item 8 — consolidated UsageInfo parser. The five near-identical JSON usage
// parsers collapse into `providers::openai::usage`. These tests drive the shared entry
// points and pin their behaviour (RED: the module does not exist yet).
// ---------------------------------------------------------------------------

#[test]
fn consolidated_openai_usage_parser() {
    use crate::infrastructure::providers::openai::usage;
    let v = serde_json::json!({
        "prompt_tokens": 12,
        "completion_tokens": 7,
        "total_tokens": 19
    });
    let u = usage::parse_openai_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 12);
    assert_eq!(u.completion_tokens, 7);
    assert_eq!(u.context_tokens, Some(12));
    assert_eq!(u.cache_read_tokens, None);
    assert_eq!(u.cache_write_tokens, None);
}

#[test]
fn consolidated_codex_usage_parser() {
    use crate::infrastructure::providers::openai::usage;
    let v = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 40,
        "input_tokens_details": { "cached_tokens": 30 }
    });
    let u = usage::parse_codex_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 70);
    assert_eq!(u.completion_tokens, 40);
    assert_eq!(u.cache_read_tokens, Some(30));
    assert_eq!(u.context_tokens, Some(100));
}

// ---------------------------------------------------------------------------
// Coverage map for the remaining (behaviour-preserving) items, whose invariants
// are pinned by existing suites rather than new tests here:
//   - Item 1 (subagent kill via libc::kill): the cascade/shutdown paths are
//     exercised by `spawn_tests.rs` and `subagent_*` suites; the change is a
//     syscall swap with no observable output change.
//   - Item 9 (streaming provider clone / router_name reset bug): pinned by
//     `anthropic_tests::streaming_task_clone_preserves_custom_router_name` — the
//     only behaviour-changing item — plus the existing streaming provider tests.
//   - Item 7 (denylist single-scan normalization): pinned by the sandbox
//     command-denylist suites, which assert unchanged block/allow semantics.
//   - Extension timer sweep: pinned by
//     `uds_ext_protocol_tests::insert_pending_sweeps_expired_entries`.
// ---------------------------------------------------------------------------
