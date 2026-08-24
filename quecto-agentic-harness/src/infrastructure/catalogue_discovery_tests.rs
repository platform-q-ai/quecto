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
