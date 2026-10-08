//! #2405: every built-in OpenAI/Codex entry declares its published window.

use super::ModelRegistry;
use crate::domain::catalogue::value_objects::catalogue::PromptLimit;

const OPENAI_PROVIDERS: [&str; 2] = ["openai-api", "openai-oauth"];

#[test]
fn every_builtin_openai_entry_declares_its_window_and_output_cap() {
    let registry = ModelRegistry::builtin();
    let entries: Vec<_> = registry
        .models()
        .iter()
        .filter(|m| OPENAI_PROVIDERS.contains(&m.provider.as_str()))
        .collect();
    assert_eq!(entries.len(), 14, "seven tiers on each OpenAI provider");
    for m in entries {
        let qualified = m.qualified_id();
        assert!(
            m.context_window_explicit,
            "{qualified} must declare its published context window"
        );
        assert!(
            m.max_tokens_explicit,
            "{qualified} must declare its output cap"
        );
        assert_eq!(
            registry.context_window_for(&qualified),
            Some(m.context_window as usize)
        );
    }
}

/// #2405 review M3: Codex (the `openai-oauth` surface) runs the GPT-5.6 and
/// GPT-6 tiers at a 272k input window (Codex's own `models.json`,
/// openai/codex@b1e72963, `context_window: 272000`): 400k less the 128k
/// output. The API keeps the published 1,050,000.
#[test]
fn the_codex_surface_runs_the_gpt_5_6_and_gpt_6_tiers_at_400k() {
    let registry = ModelRegistry::builtin();
    for id in [
        "gpt-6-astra",
        "gpt-6-sol",
        "gpt-6.1-sol",
        "gpt-6-luna",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
    ] {
        let oauth = registry.find("openai-oauth", id).expect("built in");
        assert_eq!(oauth.context_window, 400_000, "openai-oauth/{id}");
        assert_eq!(oauth.max_tokens, 128_000, "openai-oauth/{id}");
        let api = registry.find("openai-api", id).expect("built in");
        assert_eq!(api.context_window, 1_050_000, "openai-api/{id}");
        assert_eq!(api.max_tokens, 128_000, "openai-api/{id}");
    }
}

/// #2405 review M2: OpenAI fixes the input limit at the window less the
/// output cap; every other provider checks prompt + requested output.
#[test]
fn builtin_openai_entries_fix_the_input_limit_and_others_share_the_window() {
    let registry = ModelRegistry::builtin();
    for m in registry.models() {
        let expected = if OPENAI_PROVIDERS.contains(&m.provider.as_str()) {
            PromptLimit::WindowLessOutputCap
        } else {
            PromptLimit::SharedWithRequest
        };
        assert_eq!(m.prompt_limit, expected, "{}", m.qualified_id());
    }
    // A model from the user's file shares its window with the request.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"acme":{"baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"local","contextWindow":131072,"maxTokens":65536}]}}}"#,
    )
    .unwrap();
    let records = ModelRegistry::load_file_records(&path).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].prompt_limit, PromptLimit::SharedWithRequest);
}
