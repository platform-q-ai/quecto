//! Catalogue credential status agrees with the composed runtime (#2451): a
//! built-in model is credentialed exactly when the runtime builds a provider
//! for its slot from the configured keys and the credential store, so the
//! TUI model selector offers every model the runtime can route, API or
//! OAuth, and only those. Offline: temp credential stores, no network.

use super::*;
use crate::application::catalogue::dto::{ModelLimits, ModelSelectionVerdict};
use crate::application::catalogue::ports::{EffortRuntime, ModelRuntime};
use crate::domain::catalogue::UnavailableReason;
use crate::domain::provider::EffortLevel;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

const BUILTIN_SLOTS: [&str; 5] = [
    "anthropic-api",
    "anthropic-oauth",
    "openai-api",
    "openai-oauth",
    "xai",
];

fn store(dir: &Path, vendor: &str, method: AuthMethod, expires_at: Option<i64>) {
    CredentialStore::new(dir)
        .store(Credential {
            provider: vendor.to_string(),
            token: format!("test-{vendor}-token"),
            method,
            expires_at,
            refresh_token: None,
            account_id: None,
        })
        .expect("temp credential store writes");
}

fn compose_with(dir: &Path, config: &Config) -> Arc<CatalogueRuntimeSnapshot> {
    compose_and_publish_runtime(config, dir, &reqwest::Client::new())
        .expect("a provider is configured, so composition succeeds")
}

/// The `list_models` wire the TUI model selector reads, by qualified id.
fn listing(dir: &Path) -> std::collections::BTreeMap<String, serde_json::Value> {
    let wire = crate::composition::catalogue::list_models_wire_for(dir);
    wire["models"]
        .as_array()
        .unwrap_or_else(|| panic!("listing has models: {wire}"))
        .iter()
        .map(|model| (model["model"].as_str().unwrap().to_string(), model.clone()))
        .collect()
}

/// Whether the TUI would let Enter switch to the listed model: it lists no
/// reason it cannot run, and the `configured` label agrees.
fn selectable(model: &serde_json::Value) -> bool {
    let reasons = model["unavailable"].as_array().expect("unavailable array");
    let configured = model["configured"].as_bool().expect("configured flag");
    assert_eq!(
        configured,
        reasons.is_empty(),
        "label and reasons agree: {model}"
    );
    configured
}

/// The built-in providers whose models the listing offers.
fn selectable_builtin_slots(dir: &Path) -> Vec<String> {
    let mut slots: Vec<String> = listing(dir)
        .values()
        .filter(|model| BUILTIN_SLOTS.contains(&model["provider"].as_str().unwrap()))
        .filter(|model| selectable(model))
        .map(|model| model["provider"].as_str().unwrap().to_string())
        .collect();
    slots.sort();
    slots.dedup();
    slots
}

/// Every built-in model is runnable exactly when the composed runtime
/// routes its provider slot, both in the generation the runtime published
/// with (what a `set_model` verdict reads) and in the listing the TUI
/// selector reads.
fn assert_listing_matches_routes(dir: &Path, runtime: &CatalogueRuntimeSnapshot) {
    let routes = runtime.provider.route_order();
    let routed = |provider: &str| routes.iter().any(|route| route == provider);
    let composed: Vec<_> = runtime
        .catalogue
        .entries()
        .iter()
        .filter(|entry| BUILTIN_SLOTS.contains(&entry.provider.id.as_str()))
        .collect();
    assert!(!composed.is_empty(), "built-ins are published");
    for entry in composed {
        assert_eq!(
            entry.model.availability.is_runnable(),
            routed(entry.provider.id.as_str()),
            "{} is runnable in the composed generation exactly when the runtime routes its provider (routes {routes:?})",
            entry.reference().qualified_id()
        );
    }
    let builtins: Vec<_> = listing(dir)
        .into_values()
        .filter(|model| BUILTIN_SLOTS.contains(&model["provider"].as_str().unwrap()))
        .collect();
    assert!(!builtins.is_empty(), "built-ins are listed");
    for model in builtins {
        let provider = model["provider"].as_str().unwrap();
        assert_eq!(
            selectable(&model),
            routed(provider),
            "{} is selectable exactly when the runtime routes {provider} (routes {routes:?})",
            model["model"]
        );
    }
}

fn verdict(dir: &Path, model: &str) -> ModelSelectionVerdict {
    crate::composition::catalogue::build_catalogue_handles(dir, None)
        .model
        .plan(model)
        .verdict
}

fn assert_runnable_on(dir: &Path, model: &str, provider: &str) {
    match verdict(dir, model) {
        ModelSelectionVerdict::Runnable { provider: on, .. } => assert_eq!(on, provider),
        other => panic!("{model} is runnable on {provider}, got {other:?}"),
    }
}

fn assert_missing_credential(dir: &Path, model: &str) {
    match verdict(dir, model) {
        ModelSelectionVerdict::NotRunnable { reasons, .. } => {
            assert_eq!(
                reasons,
                vec![UnavailableReason::MissingCredential],
                "{model}"
            )
        }
        other => panic!("{model} lacks a credential, got {other:?}"),
    }
}

fn config_with_keys(openai: &str, anthropic: &str) -> Config {
    let mut config = Config::default();
    config.providers.openai.api_key = openai.to_string();
    config.providers.anthropic.api_key = anthropic.to_string();
    config
}

#[test]
fn anthropic_oauth_alone_offers_exactly_the_anthropic_oauth_models() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "anthropic", AuthMethod::OAuth, Some(i64::MAX));
    let runtime = compose_with(tmp.path(), &Config::default());

    assert_eq!(selectable_builtin_slots(tmp.path()), ["anthropic-oauth"]);
    assert_listing_matches_routes(tmp.path(), &runtime);
    assert_runnable_on(
        tmp.path(),
        "anthropic-oauth/claude-opus-5-5",
        "anthropic-oauth",
    );
    // An OAuth sign-in is not an API key: the runtime builds no API slot.
    assert_missing_credential(tmp.path(), "anthropic-api/claude-opus-5-5");
}

#[test]
fn openai_oauth_alone_offers_exactly_the_openai_oauth_models() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "openai", AuthMethod::OAuth, Some(i64::MAX));
    let runtime = compose_with(tmp.path(), &Config::default());

    assert_eq!(selectable_builtin_slots(tmp.path()), ["openai-oauth"]);
    assert_listing_matches_routes(tmp.path(), &runtime);
    assert_runnable_on(tmp.path(), "openai-oauth/gpt-6-sol", "openai-oauth");
    assert_missing_credential(tmp.path(), "openai-api/gpt-6-sol");
}

#[test]
fn an_expired_oauth_token_still_offers_its_models_because_the_runtime_refreshes_it_lazily() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "anthropic", AuthMethod::OAuth, Some(0));
    let runtime = compose_with(tmp.path(), &Config::default());

    assert_eq!(selectable_builtin_slots(tmp.path()), ["anthropic-oauth"]);
    assert_listing_matches_routes(tmp.path(), &runtime);
}

#[test]
fn stored_api_tokens_alone_offer_exactly_the_api_models() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "anthropic", AuthMethod::Token, None);
    store(tmp.path(), "openai", AuthMethod::Token, Some(i64::MAX));
    let runtime = compose_with(tmp.path(), &Config::default());

    assert_eq!(
        selectable_builtin_slots(tmp.path()),
        ["anthropic-api", "openai-api"]
    );
    assert_listing_matches_routes(tmp.path(), &runtime);
    assert_runnable_on(tmp.path(), "anthropic-api/claude-sonnet-5", "anthropic-api");
    assert_missing_credential(tmp.path(), "anthropic-oauth/claude-sonnet-5");
}

#[test]
fn configured_api_keys_offer_the_api_models_without_any_stored_credential() {
    let tmp = tempfile::tempdir().unwrap();
    let runtime = compose_with(tmp.path(), &config_with_keys("sk-openai", "sk-anthropic"));

    assert_eq!(
        selectable_builtin_slots(tmp.path()),
        ["anthropic-api", "openai-api"]
    );
    assert_listing_matches_routes(tmp.path(), &runtime);
    assert_runnable_on(tmp.path(), "openai-api/gpt-5.6-luna", "openai-api");
}

#[test]
fn an_expired_stored_api_token_offers_nothing_for_its_slot() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "anthropic", AuthMethod::Token, Some(0));
    let runtime = compose_with(tmp.path(), &config_with_keys("sk-openai", ""));

    assert_eq!(selectable_builtin_slots(tmp.path()), ["openai-api"]);
    assert_listing_matches_routes(tmp.path(), &runtime);
}

#[test]
fn every_slot_signed_in_offers_every_builtin_slot_the_runtime_routes() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "anthropic", AuthMethod::OAuth, Some(i64::MAX));
    store(tmp.path(), "openai", AuthMethod::OAuth, Some(i64::MAX));
    store(tmp.path(), "xai", AuthMethod::OAuth, Some(i64::MAX));
    let runtime = compose_with(tmp.path(), &config_with_keys("sk-openai", "sk-anthropic"));

    assert_eq!(selectable_builtin_slots(tmp.path()), BUILTIN_SLOTS);
    assert_listing_matches_routes(tmp.path(), &runtime);
    assert_runnable_on(tmp.path(), "xai/grok-4.7", "xai");
}

#[test]
fn no_credentials_offer_no_builtin_model() {
    let tmp = tempfile::tempdir().unwrap();
    // Nothing composes without a provider, and with no router nothing is
    // offered.
    assert!(
        compose_and_publish_runtime(&Config::default(), tmp.path(), &reqwest::Client::new())
            .is_err()
    );
    assert!(selectable_builtin_slots(tmp.path()).is_empty());
}

#[test]
fn an_unreadable_credential_store_offers_no_builtin_model_without_panicking() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("credentials.json"), "not json").unwrap();
    // The runtime cannot read the sign-ins its OAuth built-ins need, so it
    // composes nothing, even with a configured key; nothing is offered.
    let error = compose_and_publish_runtime(
        &config_with_keys("sk-openai", ""),
        tmp.path(),
        &reqwest::Client::new(),
    )
    .expect_err("an unreadable credential store fails composition");
    assert!(error.error.contains("credentials"), "{}", error.error);
    assert!(selectable_builtin_slots(tmp.path()).is_empty());
}

#[test]
fn a_signed_in_vendor_does_not_credit_a_keyless_custom_provider_on_its_wire() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "anthropic", AuthMethod::OAuth, Some(i64::MAX));
    store(tmp.path(), "openai", AuthMethod::Token, None);
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{
            "keyed":{"api":"anthropic-messages","apiKey":"sk-keyed",
                "models":[{"id":"keyed-model"}]},
            "keyless":{"api":"anthropic-messages",
                "models":[{"id":"keyless-model"}]}
        }}"#,
    )
    .unwrap();
    let runtime = compose_with(tmp.path(), &Config::default());
    assert_listing_matches_routes(tmp.path(), &runtime);

    let listed = listing(tmp.path());
    assert!(
        selectable(&listed["keyed/keyed-model"]),
        "its own key counts"
    );
    assert!(
        !selectable(&listed["keyless/keyless-model"]),
        "a vendor sign-in is not a custom provider's key"
    );
    assert_runnable_on(tmp.path(), "keyed/keyed-model", "keyed");
    assert_missing_credential(tmp.path(), "keyless/keyless-model");
}

#[test]
fn a_credential_stored_after_composition_is_offered_once_the_runtime_is_recomposed() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "anthropic", AuthMethod::OAuth, Some(i64::MAX));
    compose_with(tmp.path(), &Config::default());
    // `quecto auth login openai` while the session runs: the running
    // runtime has no openai-oauth provider, so the catalogue does not
    // offer its models either.
    store(tmp.path(), "openai", AuthMethod::OAuth, Some(i64::MAX));
    assert_eq!(selectable_builtin_slots(tmp.path()), ["anthropic-oauth"]);

    let runtime = compose_with(tmp.path(), &Config::default());
    assert_eq!(
        selectable_builtin_slots(tmp.path()),
        ["anthropic-oauth", "openai-oauth"]
    );
    assert_listing_matches_routes(tmp.path(), &runtime);
}

/// A session loop over the published runtime: route checks go to the
/// composed router itself.
struct SessionLoop {
    model: String,
    limits: ModelLimits,
    effort: Option<EffortLevel>,
    provider: Arc<dyn LlmProvider>,
}

impl EffortRuntime for SessionLoop {
    fn effort(&self) -> Option<EffortLevel> {
        self.effort
    }
    fn startup_effort(&self) -> Option<EffortLevel> {
        None
    }
    fn apply_effort(&mut self, level: Option<EffortLevel>) {
        self.effort = level;
    }
}

impl ModelRuntime for SessionLoop {
    fn model(&self) -> &str {
        &self.model
    }
    fn apply_model(&mut self, model: String, limits: ModelLimits) {
        self.model = model;
        self.limits = limits;
    }
    fn route_check(&self, model: &str) -> crate::application::providers::ports::RouteCheck {
        self.provider.route_check(model)
    }
}

#[test]
fn change_active_model_switches_to_a_signed_in_builtin_with_a_runnable_verdict() {
    let tmp = tempfile::tempdir().unwrap();
    store(tmp.path(), "anthropic", AuthMethod::OAuth, Some(i64::MAX));
    store(tmp.path(), "openai", AuthMethod::OAuth, Some(i64::MAX));
    let runtime = compose_with(tmp.path(), &Config::default());
    let mut session = SessionLoop {
        model: "anthropic-oauth/claude-opus-5-5".to_string(),
        limits: ModelLimits::default(),
        effort: None,
        provider: runtime.provider.clone(),
    };
    let handles = crate::composition::catalogue::build_catalogue_handles(tmp.path(), None);

    let switched = handles
        .model
        .execute(&mut session, "openai-oauth/gpt-6-sol")
        .expect("a signed-in built-in switches");

    assert_eq!(session.model, "openai-oauth/gpt-6-sol");
    assert_eq!(session.limits.max_output_tokens, Some(128_000));
    match switched.plan.verdict {
        ModelSelectionVerdict::Runnable { provider, .. } => assert_eq!(provider, "openai-oauth"),
        other => panic!("the switch is runnable, got {other:?}"),
    }
}

#[test]
fn a_models_json_key_under_a_builtin_slot_name_offers_that_slots_builtins() {
    let tmp = tempfile::tempdir().unwrap();
    // No configured key and no stored token: the runtime builds `openai-api`
    // from the models.json key and routes every `openai-api/...` id to it.
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai-api":{"api":"openai-completions",
            "baseUrl":"https://api.openai.com/v1","apiKey":"sk-file",
            "models":[{"id":"gpt-5.6-luna"}]}}}"#,
    )
    .unwrap();
    let runtime = compose_with(tmp.path(), &Config::default());

    assert_eq!(selectable_builtin_slots(tmp.path()), ["openai-api"]);
    assert_listing_matches_routes(tmp.path(), &runtime);
    assert_runnable_on(tmp.path(), "openai-api/gpt-6-sol", "openai-api");
}

#[test]
fn an_openai_compatible_endpoint_under_a_builtin_slot_name_offers_that_slots_builtins() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.providers.openai_compatible.endpoints = vec![
        serde_json::from_value(serde_json::json!({
            "prefix": "anthropic-api",
            "api_key": "sk-endpoint",
            "api_base": "https://gateway.example.test/v1",
        }))
        .unwrap(),
    ];
    let runtime = compose_with(tmp.path(), &config);

    assert_eq!(selectable_builtin_slots(tmp.path()), ["anthropic-api"]);
    assert_listing_matches_routes(tmp.path(), &runtime);
    assert_runnable_on(tmp.path(), "anthropic-api/claude-sonnet-5", "anthropic-api");
}
