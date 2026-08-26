//! Tests for the CLI `models discover` adapter (epic #1193, slice 4): the
//! command drives the application refresh use case and owns no discovery
//! behaviour itself.

use super::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx_for(dir: &std::path::Path) -> CliContext {
    CliContext {
        base_dir: Some(dir.to_path_buf()),
        ..Default::default()
    }
}

#[test]
fn cmd_models_discover_success_reports_count_and_accepts_interval() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "one"}, {"id": "two"}]
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let registry = serde_json::json!({"providers": {"local": {
            "api": "openai-completions",
            "baseUrl": format!("{}/v1", server.uri()),
            "apiKey": "test-token",
            "models": []
        }}});
        std::fs::write(tmp.path().join("models.json"), registry.to_string()).unwrap();
        let ctx = ctx_for(tmp.path());
        let mut stdout = String::new();
        let mut stderr = String::new();

        let ctx_for_discovery = ctx.clone();
        let (code, stdout) = tokio::task::spawn_blocking(move || {
            let code = cmd_models(
                &ctx_for_discovery,
                &[
                    "discover".to_string(),
                    "local".to_string(),
                    "--interval".to_string(),
                    "1".to_string(),
                ],
                &mut stdout,
                &mut stderr,
            );
            (code, stdout)
        })
        .await
        .unwrap();
        assert_eq!(code, 0);
        assert!(
            stdout.contains("Discovered 2 model(s) for provider local"),
            "got: {stdout}"
        );

        // Discovery persists a source cache — never a rewritten models.json.
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join("models.json")).unwrap())
                .unwrap();
        assert_eq!(after, registry, "models.json must stay user-owned");
        let cache = tmp.path().join("discovered").join("local.json");
        assert!(cache.is_file(), "discovery must persist the source cache");
    });
}

#[test]
fn cmd_models_usage_and_discover_error_paths_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_for(tmp.path());
    let mut stdout = String::new();
    let mut stderr = String::new();

    assert_eq!(cmd_models(&ctx, &[], &mut stdout, &mut stderr), 1);
    assert!(stderr.contains("Usage: quecto models discover"));

    stderr.clear();
    assert_eq!(
        cmd_models(&ctx, &["discover".to_string()], &mut stdout, &mut stderr),
        1
    );
    assert!(stderr.contains("Usage: quecto models discover"));

    // No models.json means the provider cannot be found to refresh.
    stderr.clear();
    assert_eq!(
        cmd_models(
            &ctx,
            &["discover".to_string(), "ghost".to_string()],
            &mut stdout,
            &mut stderr,
        ),
        1
    );
    assert!(stderr.contains("models discover failed"), "got: {stderr}");
    assert!(stderr.contains("ghost"), "got: {stderr}");
}

#[test]
fn cmd_models_discover_option_validation_branches_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx_for(tmp.path());
    let mut stdout = String::new();
    let mut stderr = String::new();

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

#[test]
fn discover_rejects_unsafe_base_urls_without_leaking_url_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), serde_json::json!({
        "providers": {
            "plain-remote": {"api": "openai-completions", "baseUrl": "http://attacker.example/v1", "models": []},
            "credentialed": {"api": "openai-completions", "baseUrl": "https://user:pass@example.test/v1", "models": []},
            "query": {"api": "openai-completions", "baseUrl": "https://example.test/v1?token=secret", "models": []},
            "fragment": {"api": "openai-completions", "baseUrl": "https://example.test/v1#secret", "models": []},
            "wrong-shape": {"api": "openai-completions", "baseUrl": "https://example.test/latest/meta-data", "models": []}
        }
    }).to_string()).unwrap();
    let ctx = ctx_for(tmp.path());

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
fn discover_rejects_oauth_non_openai_and_non_string_api_providers() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        serde_json::json!({"providers": {
            "oauthy": {
                "api": "openai-completions",
                "baseUrl": "https://example.test/v1",
                "auth": {"mode": "oauth"},
                "models": []
            },
            "anthropic-api": {"api": "anthropic-messages", "models": []},
            "broken": {"api": 123, "baseUrl": "https://example.test/v1", "models": []}
        }})
        .to_string(),
    )
    .unwrap();
    let ctx = ctx_for(tmp.path());

    let err = discover_once(&ctx, "oauthy").unwrap_err();
    assert!(err.contains("oauth"), "unexpected error: {err}");

    let err = discover_once(&ctx, "anthropic-api").unwrap_err();
    assert!(
        err.contains("model listing"),
        "unsupported reason must be actionable: {err}"
    );

    let err = discover_once(&ctx, "broken").unwrap_err();
    assert!(
        err.contains("api must be a string"),
        "unexpected error: {err}"
    );
}

/// Guard (epic #1193, slice 4 acceptance): the CLI models adapter performs no
/// HTTP, parses no registry data, and persists nothing itself — those
/// behaviours live behind the application refresh use case.
#[test]
fn cli_models_adapter_owns_no_discovery_mechanics() {
    let source = include_str!("models.rs");
    for forbidden in [
        "reqwest",
        "atomic_write",
        "models.json",
        "fetch",
        "read_registry",
        "resolve_registry_value",
    ] {
        assert!(
            !source.contains(forbidden),
            "interface/cli/models.rs must not mention '{forbidden}': discovery \
             mechanics belong to infrastructure behind the refresh use case"
        );
    }
}
