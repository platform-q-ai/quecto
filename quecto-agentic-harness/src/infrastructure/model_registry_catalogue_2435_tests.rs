//! #2435: the built-in catalogue offers only current models, each under
//! only the auth modes that can use it; an older model stays usable by
//! declaring it in `models.json`.

use super::builtin_tables::{BuiltinModel, BuiltinProvider, Offered, vendor_specs};
use super::{AuthMode, ModelRegistry, ProviderApi};

/// Every OpenAI model the built-in catalogue keeps: GPT-5.6 and later.
const OPENAI_KEPT: [&str; 7] = [
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6.1-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];

/// Every Anthropic model the built-in catalogue keeps: Claude 5 and later.
const ANTHROPIC_KEPT: [&str; 6] = [
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-opus-5-5",
    "claude-sonnet-5-5",
];

fn listed(provider: &str) -> Vec<String> {
    ModelRegistry::builtin()
        .models()
        .iter()
        .filter(|m| m.provider == provider)
        .map(|m| m.id.clone())
        .collect()
}

#[test]
fn every_openai_provider_lists_only_gpt_5_6_and_later() {
    for provider in ["openai-api", "openai-oauth"] {
        assert_eq!(listed(provider), OPENAI_KEPT, "{provider}");
    }
}

#[test]
fn every_anthropic_provider_lists_only_claude_5_and_later() {
    for provider in ["anthropic-api", "anthropic-oauth"] {
        assert_eq!(listed(provider), ANTHROPIC_KEPT, "{provider}");
    }
}

#[test]
fn the_default_model_is_built_in_on_both_openai_providers() {
    let default = crate::infrastructure::config::AgentDefaults::default().model;
    assert!(
        OPENAI_KEPT.contains(&default.as_str()),
        "the default `{default}` must be a kept built-in"
    );
    let registry = ModelRegistry::builtin();
    for provider in ["openai-api", "openai-oauth"] {
        assert!(registry.find(provider, &default).is_some(), "{provider}");
    }
}

#[test]
fn a_known_per_auth_exclusion_is_never_listed() {
    let providers = [
        BuiltinProvider {
            provider: "vendor-api",
            api: ProviderApi::OpenAiCompletions,
            auth: AuthMode::ApiKey,
            oauth: None,
            label: "API key",
        },
        BuiltinProvider {
            provider: "vendor-oauth",
            api: ProviderApi::OpenAiCompletions,
            auth: AuthMode::OAuth,
            oauth: Some("vendor"),
            label: "OAuth",
        },
    ];
    let models = [
        BuiltinModel {
            id: "everywhere",
            name: "Everywhere",
            offered: Offered::BOTH,
        },
        BuiltinModel {
            id: "api-only",
            name: "API only",
            offered: Offered {
                api_key: true,
                oauth: false,
            },
        },
        BuiltinModel {
            id: "oauth-only",
            name: "OAuth only",
            offered: Offered {
                api_key: false,
                oauth: true,
            },
        },
    ];
    let rows: Vec<(&str, &str, String)> = vendor_specs(&providers, &models)
        .into_iter()
        .map(|row| (row.provider, row.id, row.name))
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "vendor-api",
                "everywhere",
                "Everywhere (API key)".to_string()
            ),
            ("vendor-api", "api-only", "API only (API key)".to_string()),
            (
                "vendor-oauth",
                "everywhere",
                "Everywhere (OAuth)".to_string()
            ),
            (
                "vendor-oauth",
                "oauth-only",
                "OAuth only (OAuth)".to_string()
            ),
        ]
    );
}

#[test]
fn offered_names_exactly_the_auth_modes_it_allows() {
    let api_only = Offered {
        api_key: true,
        oauth: false,
    };
    let oauth_only = Offered {
        api_key: false,
        oauth: true,
    };
    assert!(Offered::BOTH.includes(AuthMode::ApiKey));
    assert!(Offered::BOTH.includes(AuthMode::OAuth));
    assert!(api_only.includes(AuthMode::ApiKey));
    assert!(!api_only.includes(AuthMode::OAuth));
    assert!(oauth_only.includes(AuthMode::OAuth));
    assert!(!oauth_only.includes(AuthMode::ApiKey));
}

/// A retired built-in stays usable: `models.json` declares it under the
/// built-in provider key, with the limits the user knows it has.
#[test]
fn models_json_can_still_declare_a_retired_model() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{
          "openai-oauth":{"auth":{"mode":"oauth","oauthProvider":"openai"},
            "models":[{"id":"gpt-5.5","contextWindow":400000,"maxTokens":128000,"input":["text","image"]}]},
          "anthropic-api":{"api":"anthropic-messages",
            "models":[{"id":"claude-opus-4-8","contextWindow":200000}]}
        }}"#,
    )
    .unwrap();
    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let gpt = registry.find("openai-oauth", "gpt-5.5").expect("declared");
    assert_eq!(gpt.auth, AuthMode::OAuth);
    assert_eq!(gpt.oauth_provider.as_deref(), Some("openai"));
    assert_eq!(
        registry.context_window_for("openai-oauth/gpt-5.5"),
        Some(400_000)
    );
    assert_eq!(
        registry.max_tokens_for("openai-oauth/gpt-5.5"),
        Some(128_000)
    );
    assert_eq!(gpt.input, vec!["text".to_string(), "image".to_string()]);
    let opus = registry
        .find("anthropic-api", "claude-opus-4-8")
        .expect("declared");
    assert_eq!(opus.api, ProviderApi::AnthropicMessages);
    assert_eq!(
        registry.context_window_for("anthropic-api/claude-opus-4-8"),
        Some(200_000)
    );
    // The kept built-ins are still there beside them.
    assert!(registry.find("openai-oauth", "gpt-6.1-sol").is_some());
}

/// A retired model declared without `input` takes text only: images are
/// sent only where a record declares them.
#[test]
fn a_retired_model_declared_without_input_takes_text_only() {
    assert_eq!(
        super::builtin_input("openai-oauth", "gpt-5.5"),
        vec!["text".to_string()]
    );
}

/// Review round 2: every kept Claude 5 built-in declares its published
/// 1M window and 128K output cap, and its price, on both auth modes.
#[test]
fn every_claude_5_built_in_declares_its_published_limits_and_price() {
    let registry = ModelRegistry::builtin();
    for provider in ["anthropic-api", "anthropic-oauth"] {
        for id in ANTHROPIC_KEPT {
            let m = registry.find(provider, id).expect("built in");
            assert_eq!(
                registry.context_window_for(&m.qualified_id()),
                Some(1_000_000),
                "{provider}/{id} window"
            );
            assert_eq!(
                registry.max_tokens_for(&m.qualified_id()),
                Some(128_000),
                "{provider}/{id} output cap"
            );
            let published = crate::domain::message::model_pricing(id).expect("priced");
            assert_eq!(
                m.cost.input,
                published.input_micro_usd_per_million as f64 / 1_000_000.0,
                "{provider}/{id} input price"
            );
            assert_eq!(
                m.cost.cache_read,
                published.cache_read_micro_usd_per_million as f64 / 1_000_000.0,
                "{provider}/{id} cache-read price"
            );
        }
    }
}

/// Review round 2: the TUI's fallback model list
/// (`quecto-tui/src/components/model_selector_builtin_models.txt`) is
/// exactly the built-in Anthropic and OpenAI rows, each under the auth
/// modes that offer it. The TUI reads that file; this test keeps it level
/// with the tables.
#[test]
fn the_tui_fallback_list_is_the_builtin_anthropic_and_openai_rows() {
    let expected: Vec<String> = ModelRegistry::builtin()
        .models()
        .iter()
        .filter(|m| {
            [
                "anthropic-api",
                "anthropic-oauth",
                "openai-api",
                "openai-oauth",
            ]
            .contains(&m.provider.as_str())
        })
        .map(|m| m.qualified_id())
        .collect();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../quecto-tui/src/components/model_selector_builtin_models.txt");
    let fixture = std::fs::read_to_string(&path).unwrap_or_default();
    let listed: Vec<String> = fixture
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    assert_eq!(
        listed,
        expected,
        "{} must list, one per line:\n{}",
        path.display(),
        expected.join("\n")
    );
}
