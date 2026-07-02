//! Behaviour-preserving efficiency cleanups for issue #996.
//!
//! These tests pin the observable behaviour that must survive the per-call
//! waste / duplication cleanups, and drive the two genuinely new/changed
//! surfaces (the consolidated usage parser and the dead `auth_method` config
//! field removal).

#![cfg(test)]

use crate::application::context_pruning::truncate_utf8_safe;
use crate::domain::audit::content_preview;
use crate::infrastructure::tools::truncate::truncate_line;

// ---------------------------------------------------------------------------
// Item 2 — bounded preview builders (char_indices().nth). These are pure
// regression guards: the refactor must preserve exact output on ASCII, long
// ASCII, and multibyte input, and must never split a UTF-8 character.
// ---------------------------------------------------------------------------

#[test]
fn content_preview_bounds_and_multibyte() {
    // Short ASCII: returned verbatim.
    assert_eq!(content_preview("hello", 100), "hello");
    // Exactly at the limit: verbatim (no ellipsis).
    let five = "abcde";
    assert_eq!(content_preview(five, 5), "abcde");
    // Long ASCII: truncated to max_chars total including the "..." suffix.
    let long = "a".repeat(500);
    let out = content_preview(&long, 100);
    assert_eq!(out.chars().count(), 100);
    assert!(out.ends_with("..."));
    assert_eq!(&out[..97], &"a".repeat(97));
    // Multibyte: must cut on a char boundary, never mid-codepoint. The kept
    // prefix must be exactly 97 intact 'é' chars followed by the ellipsis — a
    // mid-codepoint cut could not reconstruct to whole 'é' characters.
    let mb = "é".repeat(500); // 2 bytes each
    let out = content_preview(&mb, 100);
    assert_eq!(out.chars().count(), 100);
    assert_eq!(out, format!("{}...", "é".repeat(97)));
}

#[test]
fn truncate_utf8_safe_bounds_and_multibyte() {
    assert_eq!(truncate_utf8_safe("hi", 10).as_ref(), "hi");
    let mb = "汉".repeat(200); // 3 bytes each
    let out = truncate_utf8_safe(&mb, 60);
    assert_eq!(out.chars().count(), 60);
    // Whole-codepoint proof: 57 intact 3-byte chars plus the ellipsis. A
    // mid-codepoint cut could not reconstruct this exact string.
    assert_eq!(out.as_ref(), format!("{}...", "汉".repeat(57)));
}

#[test]
fn truncate_line_bounds_and_multibyte() {
    let (out, trunc) = truncate_line("short", 500);
    assert_eq!(out, "short");
    assert!(!trunc);

    let mb = "ω".repeat(1000);
    let (out, trunc) = truncate_line(&mb, 500);
    assert!(trunc);
    assert!(out.ends_with("... [truncated]"));
    // 500 kept chars + the suffix.
    assert_eq!(out.chars().count(), 500 + "... [truncated]".chars().count());
}

// ---------------------------------------------------------------------------
// Item 8 — consolidated UsageInfo parser. The five near-identical JSON usage
// parsers collapse into `providers::usage`. These tests drive the shared entry
// points and pin their behaviour (RED: the module does not exist yet).
// ---------------------------------------------------------------------------

#[test]
fn consolidated_openai_usage_parser() {
    use crate::infrastructure::providers::usage;
    let v = serde_json::json!({
        "prompt_tokens": 12,
        "completion_tokens": 7,
        "total_tokens": 19
    });
    let u = usage::parse_openai_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 12);
    assert_eq!(u.completion_tokens, 7);
    assert_eq!(u.context_tokens, Some(19));
    assert_eq!(u.cache_read_tokens, None);
    assert_eq!(u.cache_write_tokens, None);
}

#[test]
fn consolidated_codex_usage_parser() {
    use crate::infrastructure::providers::usage;
    let v = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 40,
        "input_tokens_details": { "cached_tokens": 30 }
    });
    let u = usage::parse_codex_usage(v.as_object().unwrap());
    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.completion_tokens, 40);
    assert_eq!(u.cache_read_tokens, Some(30));
    assert_eq!(u.context_tokens, None);
}

// ---------------------------------------------------------------------------
// Item 10 — dead `auth_method` config fields removed. Old configs that still
// carry the key must continue to load (serde ignores unknown keys), and the
// field must no longer be part of the struct's public surface.
// ---------------------------------------------------------------------------

#[test]
fn provider_entry_loads_ignoring_dead_auth_method() {
    use crate::infrastructure::config::ProviderEntry;
    // A legacy config blob that still carries the removed `auth_method` key.
    let json =
        r#"{ "api_key": "sk-x", "api_base": "https://example.test", "auth_method": "api_key" }"#;
    let entry: ProviderEntry = serde_json::from_str(json).expect("legacy config must still load");
    assert_eq!(entry.api_key, "sk-x");
    assert_eq!(entry.api_base, "https://example.test");
    // The removal itself is enforced at COMPILE time, not by the asserts above
    // (those passed before the change too, since `auth_method` used to be a
    // known field). This exhaustive struct literal fails to build if the field
    // is reintroduced.
    let _explicit = ProviderEntry {
        api_key: "k".into(),
        api_base: "b".into(),
        disable_codex_routing: false,
    };
}

// ---------------------------------------------------------------------------
// Coverage map for the remaining (behaviour-preserving) items, whose invariants
// are pinned by existing suites rather than new tests here:
//   - Item 1 (subagent kill via libc::kill): the cascade/shutdown paths are
//     exercised by `spawn_tests.rs` and `subagent_*` suites; the change is a
//     syscall swap with no observable output change.
//   - Item 9 (streaming provider clone / router_name reset bug): pinned by
//     `anthropic_tests::for_streaming_task_preserves_custom_router_name` — the
//     only behaviour-changing item — plus the existing streaming provider tests.
//   - Item 7 (denylist single-scan normalization): pinned by the sandbox
//     command-denylist suites, which assert unchanged block/allow semantics.
//   - Extension timer sweep: pinned by
//     `uds_ext_protocol_tests::insert_pending_sweeps_expired_entries`.
// ---------------------------------------------------------------------------
