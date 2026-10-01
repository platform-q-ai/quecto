//! #2405: every built-in OpenAI/Codex entry declares its published window.

use super::ModelRegistry;

const OPENAI_PROVIDERS: [&str; 2] = ["openai-api", "openai-oauth"];

/// Ids whose window OpenAI does not publish (no model page, 2026-10-01).
/// They keep today's behaviour: no declared window, the configured budget.
const UNCONFIRMED: [&str; 2] = ["gpt-5.5-mini", "gpt-5.5-nano"];

#[test]
fn every_builtin_openai_entry_declares_a_window_unless_unconfirmed() {
    let registry = ModelRegistry::builtin();
    let entries: Vec<_> = registry
        .models()
        .iter()
        .filter(|m| OPENAI_PROVIDERS.contains(&m.provider.as_str()))
        .collect();
    assert!(
        entries.len() >= 26,
        "the built-in OpenAI listings are present"
    );
    for m in entries {
        let qualified = m.qualified_id();
        if UNCONFIRMED.contains(&m.id.as_str()) {
            assert!(
                !m.context_window_explicit,
                "{qualified} has no published window and must not declare one"
            );
        } else {
            assert!(
                m.context_window_explicit,
                "{qualified} must declare its published context window"
            );
        }
    }
}

#[test]
fn the_codex_and_gpt_5_5_entries_carry_the_published_values() {
    let registry = ModelRegistry::builtin();
    // (provider, id, window, max output)
    let expected = [
        // developers.openai.com/api/docs/models/gpt-5.5
        ("openai-api", "gpt-5.5", 1_050_000, Some(128_000)),
        // openai.com/index/introducing-gpt-5-5: 400K in Codex.
        ("openai-oauth", "gpt-5.5", 400_000, Some(128_000)),
        // developers.openai.com/api/docs/models/gpt-5.3-codex
        ("openai-api", "gpt-5.3-codex", 400_000, Some(128_000)),
        ("openai-oauth", "gpt-5.3-codex", 400_000, Some(128_000)),
        // developers.openai.com/api/docs/models/gpt-5.2-codex
        ("openai-api", "gpt-5.2-codex", 400_000, Some(128_000)),
        ("openai-oauth", "gpt-5.2-codex", 400_000, Some(128_000)),
        // openai.com/index/introducing-gpt-5-3-codex-spark: 128k, no
        // published output cap.
        ("openai-api", "gpt-5.3-codex-spark", 128_000, None),
        ("openai-oauth", "gpt-5.3-codex-spark", 128_000, None),
    ];
    for (provider, id, window, max_output) in expected {
        let m = registry
            .find(provider, id)
            .unwrap_or_else(|| panic!("{provider}/{id} should be built in"));
        assert_eq!(m.context_window, window, "{provider}/{id} window");
        assert!(m.context_window_explicit, "{provider}/{id} window explicit");
        assert_eq!(
            m.max_tokens_explicit.then_some(m.max_tokens),
            max_output,
            "{provider}/{id} output cap"
        );
        assert_eq!(
            registry.context_window_for(&format!("{provider}/{id}")),
            Some(window as usize)
        );
    }
}
