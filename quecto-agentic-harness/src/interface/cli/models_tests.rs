use super::*;
use crate::infrastructure::catalogue_discovery::{
    MAX_MODEL_DISCOVERY_MODELS, MAX_MODEL_DISCOVERY_RESPONSE_BYTES, discover_models_url,
    discover_once as discovery_discover_once, discover_once_with as discovery_discover_once_with,
    fetch_openai_models, format_reqwest_error, redact_url_for_error,
};

fn discover_once(ctx: &CliContext, provider_key: &str) -> Result<usize, String> {
    discovery_discover_once(&ctx.base_dir(), provider_key)
}

fn discover_once_with<F, W>(
    ctx: &CliContext,
    provider_key: &str,
    fetch: F,
    publish: W,
) -> Result<usize, String>
where
    F: FnOnce(&str, Option<&str>) -> Result<Vec<serde_json::Value>, String>,
    W: FnOnce(&std::path::Path, &[u8]) -> Result<(), String>,
{
    discovery_discover_once_with(&ctx.base_dir(), provider_key, fetch, publish)
}
use serde_json::Value;
use wiremock::matchers::{header, method, path};

#[test]
fn cmd_models_discover_success_reports_count_and_accepts_interval() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "one"}, {"id": "two"}]
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("models.json"),
            serde_json::json!({"providers": {"local": {
                "api": "openai-completions",
                "baseUrl": format!("{}/v1", server.uri()),
                "models": []
            }}})
            .to_string(),
        )
        .unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let mut stdout = String::new();
        let mut stderr = String::new();

        let ctx_for_discovery = ctx.clone();
        let code = tokio::task::spawn_blocking(move || {
            cmd_models(
                &ctx_for_discovery,
                &[
                    "discover".to_string(),
                    "local".to_string(),
                    "--interval".to_string(),
                    "1".to_string(),
                ],
                &mut stdout,
                &mut stderr,
            )
        })
        .await
        .unwrap();
        assert_eq!(code, 0);
    });
}

#[test]
fn cmd_models_usage_and_discover_error_paths_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    assert_eq!(cmd_models(&ctx, &[], &mut stdout, &mut stderr), 1);
    assert!(stderr.contains("Usage: quecto models discover"));
    assert!(stdout.is_empty());

    stdout.clear();
    stderr.clear();
    assert_eq!(
        cmd_models(&ctx, &["discover".to_string()], &mut stdout, &mut stderr),
        1
    );
    assert!(stderr.contains("Usage: quecto models discover"));

    std::fs::write(
        tmp.path().join("models.json"),
        serde_json::json!({"providers": {"bad": {
            "api": "anthropic-messages",
            "baseUrl": "https://example.test/v1",
            "models": []
        }}})
        .to_string(),
    )
    .unwrap();
    stdout.clear();
    stderr.clear();
    assert_eq!(
        cmd_models(
            &ctx,
            &["discover".to_string(), "bad".to_string()],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert!(stderr.contains("models discover failed"));
    assert!(stderr.contains("not an openai-completions provider"));
}

#[test]
fn cmd_models_discover_option_validation_branches_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let mut stdout = String::new();
    let mut stderr = String::new();

    assert_eq!(
        cmd_models(
            &ctx,
            &[
                "discover".to_string(),
                "provider".to_string(),
                "--watch".to_string(),
                "--interval".to_string(),
            ],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert!(stderr.contains("Unknown models discover option: --interval"));

    stderr.clear();

    assert_eq!(
        cmd_models(
            &ctx,
            &[
                "discover".to_string(),
                "provider".to_string(),
                "--interval".to_string(),
                "0".to_string(),
            ],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert!(stderr.contains("at least 1 second"));

    stderr.clear();
    assert_eq!(
        cmd_models(
            &ctx,
            &[
                "discover".to_string(),
                "provider".to_string(),
                "--interval".to_string(),
                "abc".to_string(),
            ],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert!(stderr.contains("integer number of seconds"));

    stderr.clear();
    assert_eq!(
        cmd_models(
            &ctx,
            &[
                "discover".to_string(),
                "provider".to_string(),
                "--bogus".to_string(),
            ],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert!(stderr.contains("Unknown models discover option"));
}

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
                {"id": "alpha", "name": "vendor"},
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
            "providers": {"local/openai": {"api": "openai-completions", "apiBase": format!("{}/v1", server.uri()), "models": []}}
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
fn discover_accepts_camel_case_api_base() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "camel"}]})))
            .mount(&server)
            .await;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("models.json"), serde_json::json!({
            "providers": {"camel-provider": {"api": "openai-completions", "apiBase": format!("{}/v1", server.uri()), "models": []}}
        }).to_string()).unwrap();
        let ctx = CliContext { base_dir: Some(tmp.path().to_path_buf()), ..Default::default() };

        let ctx_for_discovery = ctx.clone();
        tokio::task::spawn_blocking(move || discover_once(&ctx_for_discovery, "camel-provider")).await.unwrap().unwrap();
        let after: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(tmp.path().join("models.json")).unwrap()).unwrap();
        assert_eq!(after["providers"]["camel-provider"]["models"], serde_json::json!([{"id": "camel", "name": "camel"}]));
    });
}

#[test]
fn discover_rejects_unsafe_base_urls_before_resolving_auth() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), serde_json::json!({
        "providers": {
            "plain-remote": {"api": "openai-completions", "baseUrl": "http://attacker.example/v1", "auth": {"mode": "apiKey", "apiKey": "$QUECTO_DISCOVERY_MUST_NOT_RESOLVE"}, "models": []},
            "credentialed": {"api": "openai-completions", "baseUrl": "https://user:pass@example.test/v1", "models": []},
            "query": {"api": "openai-completions", "baseUrl": "https://example.test/v1?token=secret", "models": []},
            "fragment": {"api": "openai-completions", "baseUrl": "https://example.test/v1#secret", "models": []},
            "wrong-shape": {"api": "openai-completions", "baseUrl": "https://example.test/latest/meta-data", "models": []}
        }
    }).to_string()).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };

    for (provider, expected) in [
        ("plain-remote", "http is allowed only for loopback hosts"),
        ("credentialed", "credentials in URL are not allowed"),
        ("query", "query and fragment are not allowed"),
        ("fragment", "query and fragment are not allowed"),
        (
            "wrong-shape",
            "must end at an OpenAI-compatible /v1 endpoint",
        ),
    ] {
        let err = discover_once(&ctx, provider).expect_err("unsafe URL must be rejected");
        assert!(
            err.contains(expected),
            "{provider} error should contain {expected:?}, got: {err}"
        );
    }
    assert!(std::env::var("QUECTO_DISCOVERY_MUST_NOT_RESOLVE").is_err());
    for provider in ["credentialed", "query", "fragment"] {
        let err = discover_once(&ctx, provider).unwrap_err();
        assert!(
            !err.contains("user:pass"),
            "error leaked credentials: {err}"
        );
        assert!(!err.contains("token=secret"), "error leaked query: {err}");
        assert!(!err.contains("#secret"), "error leaked fragment: {err}");
    }
}

#[test]
fn discovery_url_policy_allows_remote_http_when_explicitly_enabled() {
    let url =
        discover_models_url("remote-http", "http://example.invalid/inference/v1", true).unwrap();
    assert_eq!(url, "http://example.invalid/inference/v1/models");
}

#[test]
fn reqwest_error_urls_are_stripped_before_formatting() {
    let err = format_reqwest_error(
        "https://example.test/v1/models",
        reqwest::blocking::get("http://127.0.0.1:1/v1/models?api_key=secret").unwrap_err(),
    );
    assert!(
        !err.contains("api_key=secret"),
        "error leaked secret URL: {err}"
    );
    assert!(
        !err.contains("127.0.0.1:1"),
        "error leaked raw reqwest URL: {err}"
    );
    assert!(err.contains("https://example.test/v1/models"));
}

#[test]
fn fetch_openai_models_rejects_oversized_body_and_catalog() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let body_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("x".repeat(MAX_MODEL_DISCOVERY_RESPONSE_BYTES + 1)),
            )
            .mount(&body_server)
            .await;
        let err = tokio::task::spawn_blocking({
            let url = format!("{}/v1/models", body_server.uri());
            move || fetch_openai_models(&url, None)
        })
        .await
        .unwrap()
        .unwrap_err();
        assert!(
            err.contains("response body exceeds"),
            "unexpected body error: {err}"
        );

        let catalog_server = MockServer::start().await;
        let data: Vec<_> = (0..=MAX_MODEL_DISCOVERY_MODELS)
            .map(|i| serde_json::json!({"id": format!("model-{i}")}))
            .collect();
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": data})),
            )
            .mount(&catalog_server)
            .await;
        let err = tokio::task::spawn_blocking({
            let url = format!("{}/v1/models", catalog_server.uri());
            move || fetch_openai_models(&url, None)
        })
        .await
        .unwrap()
        .unwrap_err();
        assert!(
            err.contains("model catalog contains more than"),
            "unexpected catalog error: {err}"
        );
    });
}

#[test]
fn discovery_rejects_non_string_api_before_fetching() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        serde_json::json!({"providers": {"broken": {
            "api": 123,
            "baseUrl": "https://example.test/v1",
            "models": []
        }}})
        .to_string(),
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };

    let err = discover_once(&ctx, "broken").unwrap_err();
    assert!(
        err.contains("api must be a string"),
        "unexpected error: {err}"
    );
}

#[test]
fn fetched_models_are_deduplicated_by_id() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "alpha", "name": "First"},
                    {"id": "alpha", "name": "Last"}
                ]
            })))
            .mount(&server)
            .await;

        let models = tokio::task::spawn_blocking({
            let url = format!("{}/v1/models", server.uri());
            move || fetch_openai_models(&url, None)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            models,
            vec![serde_json::json!({"id": "alpha", "name": "Last"})]
        );
    });
}

#[test]
fn discovery_merges_into_registry_reread_after_fetch_and_uses_publisher() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        serde_json::json!({"providers": {"target": {
        "api": "openai-completions", "baseUrl": "https://example.test/v1", "models": []
    }, "other": {"models": [{"id": "before"}]}}})
        .to_string(),
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let published = std::cell::Cell::new(false);

    discover_once_with(
        &ctx,
        "target",
        |_url, _auth| {
            let mut latest: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            latest["providers"]["other"]["models"] = serde_json::json!([{"id": "concurrent"}]);
            std::fs::write(&path, serde_json::to_vec(&latest).unwrap()).unwrap();
            Ok(vec![serde_json::json!({"id": "new", "name": "new"})])
        },
        |publish_path, bytes| {
            published.set(true);
            crate::infrastructure::atomic_write::atomic_write(publish_path, bytes, Some(0o600))
                .map_err(|e| e.to_string())
        },
    )
    .unwrap();

    let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        published.get(),
        "discovery did not invoke its atomic publisher seam"
    );
    assert_eq!(after["providers"]["other"]["models"][0]["id"], "concurrent");
    assert_eq!(after["providers"]["target"]["models"][0]["id"], "new");
}

#[test]
fn error_urls_are_redacted() {
    assert_eq!(
        redact_url_for_error("https://user:pass@example.com/v1/models?token=secret#fragment"),
        "https://example.com/v1/models"
    );
}
