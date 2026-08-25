use super::*;

use quecto::domain::error::DomainError;
use quecto::infrastructure::providers::retry::RetryingProvider;
use quecto::interface::cli::build_agent_provider;

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Write a models.json registry to the scenario base dir, plus a minimal
/// config.json so the UDS agent path (list_models scenario) can load a config.
fn write_registry(world: &QuectoWorld, registry: serde_json::Value) {
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("no base dir — add 'Given a temp base directory'");
    std::fs::write(
        base.join("models.json"),
        serde_json::to_string_pretty(&registry).expect("serialize models.json"),
    )
    .expect("write models.json");
    let config_path = base.join("config.json");
    if !config_path.exists() {
        std::fs::write(
            &config_path,
            r#"{"providers":{"anthropic":{"api_key":"sk-ant-config"}}}"#,
        )
        .expect("write config.json");
    }
}

/// Load the scenario config (from config.json if present, else an empty config)
/// and build the agent provider, recording either the router or the error.
fn build_provider(world: &mut QuectoWorld) {
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");
    let config_path = base.join("config.json");
    let config = if config_path.exists() {
        Config::load_with_env(config_path.to_str().unwrap_or(""), &HashMap::new())
            .expect("load config.json")
    } else {
        serde_json::from_str::<Config>("{}").expect("default config")
    };
    let http_client = reqwest::Client::new();
    match build_agent_provider(&config, &base, &http_client) {
        Ok(provider) => world.provider = Some(provider),
        Err(e) => world.provider_build_error = Some(e),
    }
}

/// Downcast a built provider (RetryingProvider wrapping a ProviderRouter) to its
/// list of registered provider names.
fn built_router_names(world: &QuectoWorld) -> Vec<String> {
    let provider = world.provider.as_ref().unwrap_or_else(|| {
        panic!(
            "agent provider not built — build failed: {:?}",
            world.provider_build_error
        )
    });
    let retrying = provider
        .as_any()
        .downcast_ref::<RetryingProvider>()
        .expect("build_agent_provider should return a RetryingProvider");
    let router = retrying
        .inner()
        .as_any()
        .downcast_ref::<ProviderRouter>()
        .expect("RetryingProvider should wrap a ProviderRouter");
    router
        .provider_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

fn anthropic_oauth_provider() -> serde_json::Value {
    serde_json::json!({
        "api": "anthropic-messages",
        "auth": { "mode": "oauth", "oauthProvider": "anthropic" },
        "models": [{ "id": "claude-opus-4-8" }]
    })
}

fn anthropic_api_provider() -> serde_json::Value {
    serde_json::json!({
        "api": "anthropic-messages",
        "baseUrl": "https://api.anthropic.com",
        "allowRemoteHttp": true,
        "auth": { "mode": "apiKey", "apiKey": "sk-ant-direct" },
        "models": [{ "id": "claude-opus-4-8" }]
    })
}

// ─── Given steps ─────────────────────────────────────────────────────────────

#[given(expr = "a stored anthropic OAuth credential")]
fn given_stored_anthropic_oauth_credential(world: &mut QuectoWorld) {
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-valid".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .expect("store anthropic OAuth credential");
}

#[given(expr = "a models registry with an anthropic-api provider using api key {string}")]
fn given_registry_anthropic_api(world: &mut QuectoWorld, api_key: String) {
    // The key is declared as a `$ENV` reference (the production models.json
    // convention). Ensure the referenced env var resolves to a non-empty value
    // so real `$ENV` interpolation in the model registry yields a usable key.
    // Only seed a placeholder when the var is unset — never clobber a real key.
    if let Some(name) = api_key.strip_prefix('$') {
        if !name.is_empty() && std::env::var(name).is_err() {
            // BDD scenarios seed a deterministic placeholder for an otherwise-unset env var.
            // SAFETY: the value is idempotent across scenarios and never clobbers a real key.
            unsafe { std::env::set_var(name, "sk-ant-env-placeholder") };
        }
    }
    write_registry(
        world,
        serde_json::json!({
            "providers": {
                "anthropic-api": {
                    "api": "anthropic-messages",
                    "baseUrl": "https://api.anthropic.com",
                    "allowRemoteHttp": true,
                    "auth": { "mode": "apiKey", "apiKey": api_key },
                    "models": [{ "id": "claude-opus-4-8" }]
                }
            }
        }),
    );
}

#[given(
    expr = "a models registry with an anthropic-oauth provider referencing oauth provider {string}"
)]
fn given_registry_anthropic_oauth(world: &mut QuectoWorld, oauth_provider: String) {
    write_registry(
        world,
        serde_json::json!({
            "providers": {
                "anthropic-oauth": {
                    "api": "anthropic-messages",
                    "auth": { "mode": "oauth", "oauthProvider": oauth_provider },
                    "models": [{ "id": "claude-opus-4-8" }]
                }
            }
        }),
    );
}

#[given(expr = "a models registry with a provider referencing oauth provider {string}")]
fn given_registry_provider_referencing_oauth(world: &mut QuectoWorld, oauth_provider: String) {
    write_registry(
        world,
        serde_json::json!({
            "providers": {
                "cohere-oauth": {
                    "api": "openai-completions",
                    "auth": { "mode": "oauth", "oauthProvider": oauth_provider },
                    "models": [{ "id": "command-r" }]
                }
            }
        }),
    );
}

#[given(expr = "a models registry with both anthropic-oauth and anthropic-api providers")]
fn given_registry_both(world: &mut QuectoWorld) {
    write_registry(
        world,
        serde_json::json!({
            "providers": {
                "anthropic-oauth": anthropic_oauth_provider(),
                "anthropic-api": anthropic_api_provider(),
            }
        }),
    );
}

// ─── When steps ──────────────────────────────────────────────────────────────

#[when("I build the agent provider")]
fn when_build_agent_provider(world: &mut QuectoWorld) {
    build_provider(world);
}

#[when("the OAuth runtime refresh helper validates missing and incomplete credentials")]
fn when_oauth_runtime_refresh_helper_validates_prerequisites(world: &mut QuectoWorld) {
    use quecto::infrastructure::oauth_runtime::{make_oauth_refresh_fn, persist_refreshed_token};

    ensure_temp_dir(world);
    let base = base_path(world);
    let store = std::sync::Arc::new(CredentialStore::new(&base));
    let refresh = make_oauth_refresh_fn();
    let rt = tokio::runtime::Runtime::new().unwrap();

    let missing = rt
        .block_on(refresh(store.clone(), "openai"))
        .unwrap_err()
        .to_string();
    world.oauth_runtime_refresh_errors.push(missing);

    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-old".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
    let missing_refresh_token = rt
        .block_on(refresh(store.clone(), "openai"))
        .unwrap_err()
        .to_string();
    world
        .oauth_runtime_refresh_errors
        .push(missing_refresh_token);

    store
        .store(Credential {
            provider: "unknown-oauth".to_string(),
            token: "access".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(1),
            refresh_token: Some("refresh".to_string()),
            account_id: None,
        })
        .unwrap();
    let missing_oauth_config = rt
        .block_on(refresh(store.clone(), "unknown-oauth"))
        .unwrap_err()
        .to_string();
    world
        .oauth_runtime_refresh_errors
        .push(missing_oauth_config);

    let failed_refresh = persist_refreshed_token(
        &store,
        "openai",
        "refresh",
        Err(DomainError::Provider(
            "synthetic refresh failure".to_string(),
        )),
    );
    assert!(failed_refresh.is_none());
    world
        .oauth_runtime_refresh_errors
        .push("failed to refresh token for openai".to_string());
}

#[when("the OAuth runtime provider factory rebuilds OpenAI and Anthropic providers after refresh")]
fn when_oauth_runtime_provider_factory_rebuilds_providers(world: &mut QuectoWorld) {
    use quecto::infrastructure::oauth_runtime::make_provider_factory;

    let client = reqwest::Client::new();
    let openai = make_provider_factory(
        "openai",
        Some("http://example.invalid/v1".to_string()),
        client.clone(),
    );
    let anthropic = make_provider_factory(
        "anthropic",
        Some("http://example.invalid".to_string()),
        client,
    );

    world
        .oauth_runtime_rebuilt_provider_names
        .push(openai("header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOiB7ImNoYXRncHRfYWNjb3VudF9pZCI6ICJhY2N0X2JkZCJ9fQ.sig").name().to_string());
    world
        .oauth_runtime_rebuilt_provider_names
        .push(anthropic("fresh-token").name().to_string());
}

// ─── Then steps ──────────────────────────────────────────────────────────────

#[then("the OAuth runtime refresh helper should report missing credential states")]
fn then_oauth_runtime_refresh_helper_reports_missing_credential_states(world: &mut QuectoWorld) {
    assert_eq!(world.oauth_runtime_refresh_errors.len(), 4);
    assert!(world.oauth_runtime_refresh_errors[0].contains("no credential found for openai"));
    assert!(world.oauth_runtime_refresh_errors[1].contains("no refresh token for openai"));
    assert!(world.oauth_runtime_refresh_errors[2].contains("no OAuth config for unknown-oauth"));
    assert!(world.oauth_runtime_refresh_errors[3].contains("failed to refresh token for openai"));
}

#[then(expr = "the OAuth runtime rebuilt provider names should be {string} and {string}")]
fn then_oauth_runtime_rebuilt_provider_names_are(
    world: &mut QuectoWorld,
    first: String,
    second: String,
) {
    assert_eq!(
        world.oauth_runtime_rebuilt_provider_names,
        vec![first, second]
    );
}

#[then(expr = "the router should expose a provider named {string}")]
fn then_router_exposes_provider(world: &mut QuectoWorld, name: String) {
    let names = built_router_names(world);
    assert!(
        names.iter().any(|n| n == &name),
        "expected router to expose provider {name:?}, got: {names:?}"
    );
}

#[then(expr = "provider construction should fail with {string}")]
fn then_provider_construction_fails(world: &mut QuectoWorld, expected: String) {
    let err = world
        .provider_build_error
        .as_ref()
        .expect("expected provider construction to fail, but it succeeded");
    assert!(
        err.contains(&expected),
        "expected build error to contain {expected:?}, got: {err:?}"
    );
}

/// Locate the `list_models` response event and return its `data.models` array.
fn list_models_response(world: &QuectoWorld) -> Vec<serde_json::Value> {
    let event = world
        .agent_events
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|v| v["type"] == "response" && v["command"] == "list_models")
        .expect("no list_models response event found in agent events");
    assert_eq!(
        event["success"], true,
        "list_models response was not successful: {event}"
    );
    event["data"]["models"]
        .as_array()
        .expect("list_models data.models should be an array")
        .clone()
}

#[then(expr = "the list_models response should mark {string} models as auth {string}")]
fn then_list_models_marks_auth(world: &mut QuectoWorld, provider: String, auth: String) {
    let models = list_models_response(world);
    let matching: Vec<&serde_json::Value> = models
        .iter()
        .filter(|m| m["provider"] == serde_json::Value::String(provider.clone()))
        .collect();
    assert!(
        !matching.is_empty(),
        "no models found for provider {provider:?} in list_models response: {models:?}"
    );
    for model in matching {
        assert_eq!(
            model["auth"],
            serde_json::Value::String(auth.clone()),
            "model {model} for provider {provider:?} should report auth {auth:?}"
        );
    }
}
