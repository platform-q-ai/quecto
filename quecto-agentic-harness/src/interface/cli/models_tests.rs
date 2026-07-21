use super::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn discover_replaces_only_target_provider_models_and_preserves_auth() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    {"id": "alpha", "owned_by": "vendor"},
                    {"id": "beta", "name": "Beta Model"}
                ]
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let other_provider = serde_json::json!({
            "api": "anthropic-messages",
            "auth": {"mode": "apiKey", "apiKey": "$ANTHROPIC_API_KEY"},
            "models": [{"id": "claude"}]
        });
        std::fs::write(
            tmp.path().join("models.json"),
            serde_json::json!({
                "providers": {
                    "openrouter": {
                        "api": "openai-completions",
                        "baseUrl": format!("{}/v1", server.uri()),
                        "apiKey": "test-token",
                        "custom": {"keep": true},
                        "models": [{"id": "alpha", "maxTokens": 1234, "contextWindow": 4096, "reasoning": true, "cost": {"input": 1.0}}]
                    },
                    "anthropic-api": other_provider.clone()
                }
            })
            .to_string(),
        )
        .unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };

        let ctx_for_discovery = ctx.clone();
        assert_eq!(
            tokio::task::spawn_blocking(move || discover_once(&ctx_for_discovery, "openrouter"))
                .await
                .unwrap()
                .unwrap(),
            2
        );
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join("models.json")).unwrap())
                .unwrap();
        assert_eq!(after["providers"]["openrouter"]["apiKey"], "test-token");
        assert_eq!(after["providers"]["openrouter"]["custom"]["keep"], true);
        assert_eq!(
            after["providers"]["openrouter"]["models"],
            serde_json::json!([
                {"id": "alpha", "name": "vendor", "maxTokens": 1234, "contextWindow": 4096, "reasoning": true, "cost": {"input": 1.0}},
                {"id": "beta", "name": "Beta Model"}
            ])
        );
        assert_eq!(after["providers"]["anthropic-api"], other_provider);
    });
}

#[test]
fn discover_writes_valid_json_and_leaves_no_temp_file_for_literal_provider_key() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "local"}]})))
            .mount(&server)
            .await;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("models.json"), serde_json::json!({
            "providers": {"local/openai": {"api": "openai-completions", "api_base": format!("{}/v1", server.uri()), "models": []}}
        }).to_string()).unwrap();
        let ctx = CliContext { base_dir: Some(tmp.path().to_path_buf()), ..Default::default() };

        let ctx_for_discovery = ctx.clone();
        tokio::task::spawn_blocking(move || discover_once(&ctx_for_discovery, "local/openai")).await.unwrap().unwrap();
        let after: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(tmp.path().join("models.json")).unwrap()).unwrap();
        assert_eq!(after["providers"]["local/openai"]["models"], serde_json::json!([{"id": "local", "name": "local"}]));
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path()).unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "atomic temp files left behind: {leftovers:?}");
    });
}

#[test]
fn discover_accepts_default_openai_api_and_rejects_oauth() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "default-api"}]})))
            .mount(&server)
            .await;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("models.json"), serde_json::json!({
            "providers": {
                "default-api": {"baseUrl": format!("{}/v1", server.uri()), "models": []},
                "oauth-api": {"api": "openai-completions", "baseUrl": format!("{}/v1", server.uri()), "auth": {"mode": "oauth", "oauthProvider": "openai"}, "models": []}
            }
        }).to_string()).unwrap();
        let ctx = CliContext { base_dir: Some(tmp.path().to_path_buf()), ..Default::default() };

        let ctx_for_discovery = ctx.clone();
        tokio::task::spawn_blocking(move || discover_once(&ctx_for_discovery, "default-api")).await.unwrap().unwrap();
        let ctx_for_discovery = ctx.clone();
        let err = tokio::task::spawn_blocking(move || discover_once(&ctx_for_discovery, "oauth-api")).await.unwrap().unwrap_err();
        assert!(err.contains("oauth auth"), "unexpected error: {err}");
    });
}

#[test]
fn error_urls_are_redacted() {
    assert_eq!(
        redact_url_for_error("https://user:pass@example.com/v1/models?token=secret"),
        "https://example.com/v1/models"
    );
}
