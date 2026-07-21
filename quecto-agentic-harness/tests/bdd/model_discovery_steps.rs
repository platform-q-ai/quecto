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

#[then(expr = "the {string} catalog should contain models {string} and {string}")]
fn then_catalog_contains(world: &mut QuectoWorld, provider: String, first: String, second: String) {
    let registry = read_registry(world);
    assert_eq!(
        registry["providers"][provider]["models"],
        serde_json::json!([
            {"id": first, "name": "vendor"},
            {"id": second, "name": "Beta Model"}
        ])
    );
}

#[then(expr = "the {string} auth and custom settings should be unchanged")]
fn then_settings_unchanged(world: &mut QuectoWorld, provider: String) {
    let registry = read_registry(world);
    assert_eq!(
        registry["providers"][&provider]["auth"],
        serde_json::json!({"mode": "apiKey", "apiKey": "test-token"})
    );
    assert_eq!(
        registry["providers"][&provider]["custom"],
        serde_json::json!({"keep": true})
    );
}

#[then(expr = "the {string} provider should be unchanged")]
fn then_other_unchanged(world: &mut QuectoWorld, provider: String) {
    let registry = read_registry(world);
    assert_eq!(
        registry["providers"][provider],
        serde_json::json!({
            "api": "anthropic-messages",
            "auth": {"mode": "apiKey", "apiKey": "$ANTHROPIC_API_KEY"},
            "models": [{"id": "claude"}]
        })
    );
}

#[then(expr = "the models registry should remain valid JSON")]
fn then_valid_json(world: &mut QuectoWorld) {
    let _: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(base_path(world).join("models.json")).unwrap(),
    )
    .unwrap();
}

#[then(expr = "no discovery temporary file should remain")]
fn then_no_tmp(world: &mut QuectoWorld) {
    let leftovers: Vec<_> = std::fs::read_dir(base_path(world))
        .unwrap()
        .filter_map(Result::ok)
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

fn read_registry(world: &QuectoWorld) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(base_path(world).join("models.json")).unwrap())
        .unwrap()
}

#[then("the discovery implementation should use the atomic write helper")]
fn then_uses_atomic_write_helper(_world: &mut QuectoWorld) {
    let source = std::fs::read_to_string("src/interface/cli/models.rs").unwrap();
    assert!(source.contains("atomic_write(&path"));
}
