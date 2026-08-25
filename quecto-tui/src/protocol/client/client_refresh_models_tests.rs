use super::Command;

#[test]
fn command_refresh_models_can_select_provider() {
    let cmd = Command::RefreshModels {
        id: Some("r".into()),
        provider: Some("openai-api".into()),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"refresh_models\""));
    assert!(json.contains("\"provider\":\"openai-api\""));
}
