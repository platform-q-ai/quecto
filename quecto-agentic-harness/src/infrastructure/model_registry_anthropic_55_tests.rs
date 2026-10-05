use super::*;

// Anthropic model overview and pricing, retrieved 2026-10-05:
// https://platform.claude.com/docs/en/about-claude/models/overview
// https://platform.claude.com/docs/en/about-claude/pricing
// Costs are standard global USD/MTok, with 5-minute cache writes.
// OAuth rows expose API-reference estimates, not subscription invoices.
fn assert_builtin_claude_5_5(
    provider: &str,
    id: &str,
    label: &str,
    auth: AuthMode,
    oauth_provider: Option<&str>,
    costs: (f64, f64, f64, f64),
) {
    let registry = ModelRegistry::builtin();
    let matches: Vec<_> = registry
        .models()
        .iter()
        .filter(|model| model.provider == provider && model.id == id)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "exactly one {provider}/{id} must be built in"
    );
    let model = matches[0];
    assert_eq!(model.provider, provider);
    assert_eq!(model.id, id);
    assert_eq!(model.qualified_id(), format!("{provider}/{id}"));
    assert_eq!(model.display_name.as_deref(), Some(label));
    assert_eq!(model.api, ProviderApi::AnthropicMessages);
    assert_eq!(model.auth, auth);
    assert_eq!(model.oauth_provider.as_deref(), oauth_provider);
    assert_eq!(model.input, vec!["text".to_string(), "image".to_string()]);
    assert_eq!(model.context_window, 1_000_000);
    assert!(model.context_window_explicit);
    assert_eq!(model.max_tokens, 128_000);
    assert!(model.max_tokens_explicit);
    assert_eq!(model.cost.input, costs.0);
    assert_eq!(model.cost.output, costs.1);
    assert_eq!(model.cost.cache_read, costs.2);
    assert_eq!(model.cost.cache_write, costs.3);
}

#[test]
fn builtin_claude_opus_5_5_api_has_sourced_metadata_and_costs() {
    assert_builtin_claude_5_5(
        "anthropic-api",
        "claude-opus-5-5",
        "Claude Opus 5.5 (API key)",
        AuthMode::ApiKey,
        None,
        (4.0, 20.0, 0.20, 5.0),
    );
}

#[test]
fn builtin_claude_opus_5_5_oauth_has_sourced_metadata_and_costs() {
    assert_builtin_claude_5_5(
        "anthropic-oauth",
        "claude-opus-5-5",
        "Claude Opus 5.5 (OAuth)",
        AuthMode::OAuth,
        Some("anthropic"),
        (4.0, 20.0, 0.20, 5.0),
    );
}

#[test]
fn builtin_claude_sonnet_5_5_api_has_sourced_metadata_and_costs() {
    assert_builtin_claude_5_5(
        "anthropic-api",
        "claude-sonnet-5-5",
        "Claude Sonnet 5.5 (API key)",
        AuthMode::ApiKey,
        None,
        (2.0, 10.0, 0.20, 2.50),
    );
}

#[test]
fn builtin_claude_sonnet_5_5_oauth_has_sourced_metadata_and_costs() {
    assert_builtin_claude_5_5(
        "anthropic-oauth",
        "claude-sonnet-5-5",
        "Claude Sonnet 5.5 (OAuth)",
        AuthMode::OAuth,
        Some("anthropic"),
        (2.0, 10.0, 0.20, 2.50),
    );
}
