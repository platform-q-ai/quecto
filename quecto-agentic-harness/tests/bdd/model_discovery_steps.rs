use super::*;
use quecto::interface::cli::run_with_output;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[given(expr = "provider {string} has auth, custom settings, and an old model")]
fn given_target_provider(world: &mut QuectoWorld, provider: String) {
    ensure_temp_dir(world);
    let registry = serde_json::json!({
        "providers": {
            provider: {
                "api": "openai-completions",
                "baseUrl": "http://127.0.0.1/placeholder/v1",
                "auth": {"mode": "apiKey", "apiKey": "test-token"},
                "custom": {"keep": true},
                "models": [{"id": "old"}]
            }
        }
    });
    std::fs::write(base_path(world).join("models.json"), registry.to_string()).unwrap();
}

#[given(expr = "provider {string} has its own auth and models")]
fn given_other_provider(world: &mut QuectoWorld, provider: String) {
    let mut registry: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(base_path(world).join("models.json")).unwrap(),
    )
    .unwrap();
    registry["providers"][provider] = serde_json::json!({
        "api": "anthropic-messages",
        "auth": {"mode": "apiKey", "apiKey": "$ANTHROPIC_API_KEY"},
        "models": [{"id": "claude"}]
    });
    std::fs::write(base_path(world).join("models.json"), registry.to_string()).unwrap();
}

#[given(expr = "provider {string} has an empty model catalog")]
fn given_empty_provider(world: &mut QuectoWorld, provider: String) {
    ensure_temp_dir(world);
    let registry = serde_json::json!({
        "providers": {
            provider: {"api": "openai-completions", "baseUrl": "http://127.0.0.1/placeholder/v1", "models": []}
        }
    });
    std::fs::write(base_path(world).join("models.json"), registry.to_string()).unwrap();
}

#[given(expr = "the OpenAI-compatible catalog for {string} returns models {string} and {string}")]
fn given_catalog_two(world: &mut QuectoWorld, provider: String, first: String, second: String) {
    mount_catalog(
        world,
        &provider,
        serde_json::json!({
            "data": [{"id": first, "owned_by": "vendor"}, {"id": second, "name": "Beta Model"}]
        }),
    );
}

#[given(expr = "the OpenAI-compatible catalog for {string} returns model {string}")]
fn given_catalog_one(world: &mut QuectoWorld, provider: String, model: String) {
    mount_catalog(
        world,
        &provider,
        serde_json::json!({"data": [{"id": model}]}),
    );
}

#[when(expr = "I discover models for provider {string}")]
fn when_discover(world: &mut QuectoWorld, provider: String) {
    world.model_discovery_registry_snapshot =
        Some(std::fs::read_to_string(base_path(world).join("models.json")).unwrap());
    let output = run_with_output(
        vec![
            "quecto".into(),
            "models".into(),
            "discover".into(),
            provider,
        ],
        &world.cli_context,
    );
    assert_eq!(output.exit_code, 0, "{}{}", output.stdout, output.stderr);
}

#[then(expr = "the {string} discovery cache should contain models {string} and {string}")]
fn then_cache_contains(world: &mut QuectoWorld, provider: String, first: String, second: String) {
    let cache = read_discovery_cache(world, &provider);
    assert_eq!(
        cache,
        serde_json::json!([
            {"id": first, "name": "vendor"},
            {"id": second, "name": "Beta Model"}
        ])
    );
}

#[then(expr = "the user-owned models registry should be unchanged by discovery")]
fn then_registry_unchanged(world: &mut QuectoWorld) {
    let before = world
        .model_discovery_registry_snapshot
        .as_deref()
        .expect("registry snapshot captured before discovery");
    let after = std::fs::read_to_string(base_path(world).join("models.json")).unwrap();
    assert_eq!(
        before, after,
        "discovery must not rewrite user-owned models.json"
    );
}

#[then(expr = "no discovery cache should exist for provider {string}")]
fn then_no_cache_for(world: &mut QuectoWorld, provider: String) {
    let path = discovery_cache_path(world, &provider);
    assert!(
        !path.exists(),
        "unexpected discovery cache at {}",
        path.display()
    );
}

#[then(expr = "the {string} discovery cache should be valid JSON")]
fn then_valid_json(world: &mut QuectoWorld, provider: String) {
    let cache = read_discovery_cache(world, &provider);
    assert!(
        cache.as_array().is_some_and(|models| !models.is_empty()),
        "discovery cache should be a non-empty JSON array, got: {cache}"
    );
}

#[then(expr = "no discovery temporary file should remain")]
fn then_no_tmp(world: &mut QuectoWorld) {
    let discovered =
        base_path(world).join(quecto::infrastructure::catalogue_discovery::DISCOVERY_CACHE_DIR);
    let leftovers: Vec<_> = std::fs::read_dir(base_path(world))
        .unwrap()
        .filter_map(Result::ok)
        .chain(
            std::fs::read_dir(&discovered)
                .into_iter()
                .flatten()
                .filter_map(Result::ok),
        )
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "atomic temp files left behind: {leftovers:?}"
    );
}

fn mount_catalog(world: &mut QuectoWorld, provider: &str, response: serde_json::Value) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;
        server
    });
    let mut registry = read_registry(world);
    registry["providers"][provider]["baseUrl"] = serde_json::json!(format!("{}/v1", server.uri()));
    std::fs::write(base_path(world).join("models.json"), registry.to_string()).unwrap();
    world._model_discovery_mock_server = Some(Box::leak(Box::new(server)));
}

fn discovery_cache_path(world: &QuectoWorld, provider: &str) -> std::path::PathBuf {
    base_path(world)
        .join(quecto::infrastructure::catalogue_discovery::DISCOVERY_CACHE_DIR)
        .join(format!("{provider}.json"))
}

fn read_discovery_cache(world: &QuectoWorld, provider: &str) -> serde_json::Value {
    let path = discovery_cache_path(world, provider);
    serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read discovery cache {}: {e}", path.display())),
    )
    .unwrap()
}

fn read_registry(world: &QuectoWorld) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(base_path(world).join("models.json")).unwrap())
        .unwrap()
}
