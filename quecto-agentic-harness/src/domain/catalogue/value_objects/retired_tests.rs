use super::*;

#[test]
fn the_retired_built_ins_are_named_under_the_providers_that_listed_them() {
    for provider in ["openai-api", "openai-oauth"] {
        for model in [
            "gpt-5.5",
            "gpt-5.5-mini",
            "gpt-5.5-nano",
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark",
            "gpt-5.2-codex",
        ] {
            assert!(retired_builtin(provider, model), "{provider}/{model}");
        }
    }
    for provider in ["anthropic-api", "anthropic-oauth"] {
        for model in [
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5",
            "claude-sonnet-4-6",
            "claude-sonnet-4-5",
        ] {
            assert!(retired_builtin(provider, model), "{provider}/{model}");
        }
    }
}

#[test]
fn nothing_else_is_retired() {
    for (provider, model) in [
        ("openai-oauth", "gpt-6.1-sol"),
        ("openai-oauth", "gpt-5.6-sol"),
        ("anthropic-api", "claude-opus-5"),
        ("openai-oauth", "claude-opus-4-8"),
        ("fireworks", "gpt-5.5"),
        ("openai-api", "gpt-5.5-turbo"),
        ("openai-api", "my-model"),
    ] {
        assert!(!retired_builtin(provider, model), "{provider}/{model}");
    }
}

/// Review round 2: the model id is matched whatever its case, as the
/// provider is.
#[test]
fn a_retired_id_in_another_case_is_retired() {
    assert!(retired_builtin("OpenAI-OAuth", "GPT-5.5"));
    assert!(retired_builtin("anthropic-api", "Claude-Opus-4-8"));
}
