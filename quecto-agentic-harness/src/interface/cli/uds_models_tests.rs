use super::list_models_data;

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
