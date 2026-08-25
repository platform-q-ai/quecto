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

#[test]
fn publication_lock_preserves_different_provider_updates_across_processes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        serde_json::json!({"providers": {
            "alpha": {"baseUrl": "https://example.test/v1", "models": []},
            "beta": {"baseUrl": "https://example.test/v1", "models": []}
        }})
        .to_string(),
    )
    .unwrap();

    let lock = ModelsJsonPublishLock::acquire(tmp.path()).unwrap();
    let alpha_dir = tmp.path().to_path_buf();
    let alpha = std::thread::spawn(move || {
        discover_once_with(
            &alpha_dir,
            "alpha",
            |_url, _auth| Ok(vec![serde_json::json!({"id": "alpha-model"})]),
            |path, bytes| atomic_write(path, bytes, Some(0o600)).map_err(|e| e.to_string()),
        )
        .unwrap();
    });
    let beta_dir = tmp.path().to_path_buf();
    let beta = std::thread::spawn(move || {
        discover_once_with(
            &beta_dir,
            "beta",
            |_url, _auth| Ok(vec![serde_json::json!({"id": "beta-model"})]),
            |path, bytes| atomic_write(path, bytes, Some(0o600)).map_err(|e| e.to_string()),
        )
        .unwrap();
    });

    std::thread::sleep(std::time::Duration::from_millis(50));
    drop(lock);
    alpha.join().unwrap();
    beta.join().unwrap();

    let published: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        published["providers"]["alpha"]["models"][0]["id"],
        "alpha-model"
    );
    assert_eq!(
        published["providers"]["beta"]["models"][0]["id"],
        "beta-model"
    );
}

#[test]
fn discover_models_url_rejects_unsafe_and_non_v1_bases_without_leaking_credentials() {
    let insecure =
        discover_models_url("open", "http://insecure.example/v1?token=secret", false).unwrap_err();
    assert!(insecure.contains("invalid baseUrl"));
    assert!(!insecure.contains("token=secret"));

    let non_v1 = discover_models_url("open", "https://example.test/api", true).unwrap_err();
    assert!(non_v1.contains("must end at an OpenAI-compatible /v1 endpoint"));
}

#[test]
fn fetch_openai_models_reports_transport_and_body_failures() {
    // Bind then drop so the port is closed: the request must fail in transport
    // rather than hang, and the error must not embed the request URL twice.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = closed.local_addr().unwrap();
    drop(closed);
    let transport = fetch_openai_models(&format!("http://{addr}/v1/models"), None).unwrap_err();
    assert!(transport.starts_with("GET http://"));
    assert!(transport.contains("failed"));

    let invalid_json =
        fetch_openai_models(&serve_models_response(200, "not json"), None).unwrap_err();
    assert!(invalid_json.contains("returned invalid JSON"));
}

#[test]
fn publish_lock_reports_unopenable_lock_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing-dir");
    let error = match ModelsJsonPublishLock::acquire(&missing) {
        Ok(_) => panic!("publication lock must not be acquired in a missing directory"),
        Err(error) => error,
    };
    assert!(error.contains("failed to open"));
    assert!(error.contains("models.json.lock"));
}
