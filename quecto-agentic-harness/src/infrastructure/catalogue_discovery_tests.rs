use std::io::Read;

use super::*;

#[test]
fn refresh_all_returns_per_provider_outcomes_and_does_not_short_circuit_unsupported_sources() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        serde_json::json!({"providers": {
            "anthropic": {"api": "anthropic-messages", "models": [{"id": "claude"}]},
            "oauth": {"api": "openai-completions", "baseUrl": "https://example.test/v1", "auth": {"mode": "oauth", "oauthProvider": "openai"}, "models": []},
            "open": {"api": "openai-completions", "baseUrl": "https://127.0.0.1:9/v1", "models": []}
        }})
        .to_string(),
    )
    .unwrap();

    let outcomes = ModelsJsonCatalogueRefreshAdapter::new(tmp.path()).refresh_all();

    assert_eq!(
        outcomes
            .iter()
            .map(|o| o.source.as_str())
            .collect::<Vec<_>>(),
        ["anthropic", "oauth", "open"]
    );
    assert!(matches!(
        outcomes[0].status,
        CatalogueRefreshStatus::Skipped { .. }
    ));
    assert!(matches!(
        outcomes[1].status,
        CatalogueRefreshStatus::Skipped { .. }
    ));
    assert!(matches!(
        outcomes[2].status,
        CatalogueRefreshStatus::Failed { .. }
    ));
}

#[test]
fn openai_discovery_fetches_publishes_and_trait_refresh_reports_missing_registry() {
    let missing = tempfile::tempdir().unwrap();
    let refresh_port: &dyn CatalogueRefreshAllPort =
        &ModelsJsonCatalogueRefreshAdapter::new(missing.path());
    let missing_outcomes = refresh_port.refresh_all_sources();
    assert_eq!(missing_outcomes.len(), 1);
    assert_eq!(missing_outcomes[0].source, "models.json");
    assert!(matches!(
        missing_outcomes[0].status,
        CatalogueRefreshStatus::Failed { .. }
    ));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0; 1024];
        let _ = stream.read(&mut buf).unwrap();
        let body = r#"{"data":[{"id":"zeta","owned_by":"owner"},{"id":"alpha","name":"Alpha"},{"id":"zeta","name":"Zed Duplicate"}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        std::io::Write::write_all(&mut stream, response.as_bytes()).unwrap();
    });

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        serde_json::json!({"providers": {
            "open": {
                "api": "openai-completions",
                "baseUrl": format!("http://{addr}/v1"),
                "allowRemoteHttp": true,
                "models": []
            }
        }})
        .to_string(),
    )
    .unwrap();

    let discovered = discover_once(tmp.path(), "open").unwrap();

    server.join().unwrap();
    assert_eq!(discovered, 2);
    let registry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let models = registry["providers"]["open"]["models"].as_array().unwrap();
    assert_eq!(models[0]["id"], "alpha");
    assert_eq!(models[1]["id"], "zeta");
}

#[test]
fn discover_once_with_reports_registry_validation_and_publish_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");

    std::fs::write(&path, "{}").unwrap();
    assert!(
        provider_keys(&path)
            .unwrap_err()
            .contains("missing providers object")
    );
    assert!(
        discover_once_with(
            tmp.path(),
            "open",
            |_url, _auth| Ok(vec![]),
            |_path, _bytes| Ok(())
        )
        .unwrap_err()
        .contains("provider 'open' not found")
    );

    std::fs::write(
        &path,
        serde_json::json!({"providers": {"open": {"api": 7, "models": []}}}).to_string(),
    )
    .unwrap();
    assert!(
        discover_once_with(
            tmp.path(),
            "open",
            |_url, _auth| Ok(vec![]),
            |_path, _bytes| Ok(())
        )
        .unwrap_err()
        .contains("api must be a string")
    );

    std::fs::write(
        &path,
        serde_json::json!({"providers": {
            "anthropic": {"api": "anthropic-messages", "baseUrl": "https://example.test/v1", "models": []},
            "oauth": {"api": "openai-completions", "baseUrl": "https://example.test/v1", "auth": {"mode": "oauth"}, "models": []},
            "open": {"api": "openai-completions", "baseUrl": "https://example.test/v1", "apiKey": "direct-token", "models": []}
        }})
        .to_string(),
    )
    .unwrap();
    assert!(
        discover_once_with(
            tmp.path(),
            "anthropic",
            |_url, _auth| Ok(vec![]),
            |_path, _bytes| Ok(())
        )
        .unwrap_err()
        .contains("is not an openai-completions provider")
    );
    assert!(
        discover_once_with(
            tmp.path(),
            "oauth",
            |_url, _auth| Ok(vec![]),
            |_path, _bytes| Ok(())
        )
        .unwrap_err()
        .contains("uses oauth auth")
    );

    let published = discover_once_with(
        tmp.path(),
        "open",
        |url, auth| {
            assert_eq!(url, "https://example.test/v1/models");
            assert_eq!(auth, Some("direct-token"));
            std::fs::write(
                &path,
                serde_json::json!({"providers": {
                    "open": {"api": "openai-completions", "baseUrl": "https://example.test/v1", "apiKey": "direct-token", "models": [{"id":"previous"}]},
                    "other": {"api": "openai-completions", "baseUrl": "https://example.test/v1", "models": [{"id":"keep"}]}
                }})
                .to_string(),
            )
            .unwrap();
            Ok(vec![json!({"id":"fresh"})])
        },
        |_path, _bytes| Err("disk full".to_string()),
    )
    .unwrap_err();
    assert!(published.contains("failed to write"));
}

#[test]
fn stale_discovery_is_discarded_when_provider_configuration_changes_before_publish() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        serde_json::json!({"providers": {"open": {
            "api": "openai-completions",
            "baseUrl": "https://old.example.test/v1",
            "models": [{"id":"old-runtime"}]
        }}})
        .to_string(),
    )
    .unwrap();

    let error = discover_once_with(
        tmp.path(),
        "open",
        |_url, _auth| {
            std::fs::write(
                &path,
                serde_json::json!({"providers": {"open": {
                    "api": "openai-completions",
                    "baseUrl": "https://new.example.test/v1",
                    "models": [{"id":"new-runtime"}]
                }}})
                .to_string(),
            )
            .unwrap();
            Ok(vec![json!({"id":"stale-old-backend"})])
        },
        |_path, _bytes| Ok(()),
    )
    .unwrap_err();

    assert!(error.contains("changed during discovery"));
    let registry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        registry["providers"]["open"]["baseUrl"],
        "https://new.example.test/v1"
    );
    assert_eq!(
        registry["providers"]["open"]["models"][0]["id"],
        "new-runtime"
    );
}

#[test]
fn fetch_openai_models_reports_http_and_payload_errors() {
    let status_error =
        fetch_openai_models(&serve_models_response(503, r#"{"error":"down"}"#), None).unwrap_err();
    assert!(status_error.contains("503"));

    let missing_data =
        fetch_openai_models(&serve_models_response(200, "{}"), Some("token")).unwrap_err();
    assert!(missing_data.contains("missing data array"));

    let missing_id = fetch_openai_models(
        &serve_models_response(200, r#"{"data":[{"name":"missing id"}]}"#),
        None,
    )
    .unwrap_err();
    assert!(missing_id.contains("model entry missing string id"));
}

fn serve_models_response(status: u16, body: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0; 1024];
        let _ = stream.read(&mut buf).unwrap();
        let response = format!(
            "HTTP/1.1 {status} Test\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        std::io::Write::write_all(&mut stream, response.as_bytes()).unwrap();
    });
    format!("http://{addr}/v1/models")
}

#[test]
fn provider_keys_are_sorted_for_deterministic_refresh_publication() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        serde_json::json!({"providers": {
            "zeta": {"models": []},
            "alpha": {"models": []},
            "middle": {"models": []}
        }})
        .to_string(),
    )
    .unwrap();

    assert_eq!(provider_keys(&path).unwrap(), ["alpha", "middle", "zeta"]);
}
