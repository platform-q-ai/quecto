use super::{list_models_data, refresh_models_data};

#[test]
fn list_models_data_serializes_registry_models() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
            tmp.path().join("models.json"),
            r#"{
              "providers": {
                "anthropicish": {
                  "api": "anthropic-messages",
                  "apiKey": "sk-a",
                  "models": [{"id":"claude-custom", "reasoning": true}]
                },
                "googleish": {
                  "api": "google-generative-ai",
                  "baseUrl": "https://google.example/v1",
                  "models": [{"id":"gemini-custom"}]
                },
                "openish": {
                  "api": "openai-completions",
                  "models": [{"id":"open-custom", "cost":{"input":1.0,"output":2.0,"cacheRead":3.0,"cacheWrite":4.0}}]
                }
              }
            }"#,
        )
        .unwrap();

    let data = list_models_data(tmp.path());
    let models = data["models"].as_array().unwrap();
    let find = |id: &str| {
        models
            .iter()
            .find(|m| m["model"].as_str().unwrap().ends_with(id))
            .unwrap()
    };

    let anthropic = find("claude-custom");
    assert_eq!(anthropic["api"], "anthropic-messages");
    assert_eq!(anthropic["auth"], "apiKey");
    assert_eq!(anthropic["configured"], true);
    assert_eq!(anthropic["reasoning"], true);

    let google = find("gemini-custom");
    assert_eq!(google["api"], "google-generative-ai");
    assert_eq!(google["configured"], true);

    let open = find("open-custom");
    assert_eq!(open["api"], "openai-completions");
    assert_eq!(open["configured"], false);
    assert_eq!(open["cost"]["cacheRead"], 3.0);
    assert_eq!(open["cost"]["cacheWrite"], 4.0);
}

#[test]
fn list_models_data_reports_registry_errors() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), "not json").unwrap();

    let data = list_models_data(tmp.path());

    assert_eq!(data["models"].as_array().unwrap().len(), 0);
    assert!(data["error"].as_str().unwrap().contains("failed to parse"));
}

/// A record the catalogue drops (domain-rejected or unmappable) must surface
/// as a wire diagnostic instead of silently vanishing from the listing.
#[test]
fn list_models_data_surfaces_rejected_and_skipped_records() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"custom":{"api":"openai-completions",
            "models":[{"id":"good"},{"id":"bad","maxTokens":0},{"id":""}]}}}"#,
    )
    .unwrap();

    let data = list_models_data(tmp.path());
    let models = data["models"].as_array().unwrap();
    assert!(
        models
            .iter()
            .any(|m| m["model"].as_str() == Some("custom/good")),
        "valid sibling records must survive"
    );
    assert!(
        !models
            .iter()
            .any(|m| m["model"].as_str() == Some("custom/bad"))
    );
    let rejected = data["rejected"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|r| r["model"].as_str() == Some("custom/bad")
                && !r["reason"].as_str().unwrap().is_empty()),
        "domain-rejected entries must carry a diagnostic: {rejected:?}"
    );
    assert!(
        rejected
            .iter()
            .any(|r| r["model"].as_str() == Some("custom/")),
        "unmappable records must carry a diagnostic: {rejected:?}"
    );
}

#[test]
fn refresh_models_data_reports_per_source_outcomes_on_the_wire() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "alpha", "name": "Alpha"}]
            })))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("models.json"),
            serde_json::json!({"providers": {
                "openrouter": {
                    "api": "openai-completions",
                    "baseUrl": format!("{}/v1", server.uri()),
                    "models": []
                },
                "anthropic-api": {"api": "anthropic-messages", "models": []}
            }})
            .to_string(),
        )
        .unwrap();

        let base_dir = tmp.path().to_path_buf();
        let data = tokio::task::spawn_blocking(move || refresh_models_data(&base_dir, None))
            .await
            .unwrap();

        let outcomes = data["outcomes"].as_array().expect("outcomes array");
        let by_source = |source: &str| {
            outcomes
                .iter()
                .find(|o| o["source"] == source)
                .unwrap_or_else(|| panic!("no outcome for {source}: {data}"))
                .clone()
        };
        assert_eq!(by_source("openrouter")["status"], "updated");
        assert_eq!(by_source("openrouter")["models"], 1);
        assert_eq!(by_source("anthropic-api")["status"], "unsupported");
        assert!(
            by_source("anthropic-api")["reason"]
                .as_str()
                .unwrap()
                .contains("model listing"),
            "unsupported reason must be actionable: {data}"
        );
        assert!(
            data["generation"].as_u64().is_some(),
            "an updating refresh must report the published generation: {data}"
        );

        // A subset refresh touches only the named source.
        let base_dir = tmp.path().to_path_buf();
        let data = tokio::task::spawn_blocking(move || {
            refresh_models_data(&base_dir, Some("anthropic-api"))
        })
        .await
        .unwrap();
        let outcomes = data["outcomes"].as_array().unwrap();
        assert_eq!(outcomes.len(), 1, "subset refresh reports one outcome");
        assert_eq!(outcomes[0]["source"], "anthropic-api");
    });
}
