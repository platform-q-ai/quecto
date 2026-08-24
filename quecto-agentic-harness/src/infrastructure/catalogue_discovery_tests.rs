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
