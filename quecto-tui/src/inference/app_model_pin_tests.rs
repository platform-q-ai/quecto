//! Pinning the selected model as a repository or global default from the
//! `/model` selector (#2024 S2): the persist scope rides on `set_model`,
//! the toast reports where the harness recorded it, a refused pin is a
//! failed switch.

use super::tui_harness::TuiHarness;
use crate::shell::keys::Key;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn command_has_string_fields(command: &str, expected: &[(&str, &str)]) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(command) else {
        return false;
    };
    expected
        .iter()
        .all(|(key, want)| value.get(*key).and_then(|v| v.as_str()) == Some(*want))
}

#[tokio::test]
async fn model_selector_tab_then_enter_sends_set_model_with_persist_and_the_toast_names_the_file() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [{ "id": "openai-api/gpt-5.6-luna", "provider": "OpenAI API" }]
    })));
    a.handle_model_selector_key(&Key::Tab);
    a.handle_model_selector_key(&Key::Enter);
    assert!(a.inference.model_selector.is_none());
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| command_has_string_fields(
            c,
            &[
                ("type", "set_model"),
                ("model", "openai-api/gpt-5.6-luna"),
                ("persist", "local"),
            ]
        )),
        "Tab+Enter should send set_model with persist local: {cmds:?}"
    );
    h.event(crate::protocol::client::Event::Response {
        id: None,
        command: "set_model".into(),
        success: true,
        data: Some(serde_json::json!({
            "selection": { "status": "ok", "provider": "openai-api", "generation": 2 },
            "persisted": { "scope": "local", "path": "/repo/.quecto/config.json" }
        })),
        error: None,
    });
    let text = h.notification_messages().join("\n");
    assert!(
        text.contains(
            "Model switched and pinned as this repo's default (/repo/.quecto/config.json); live tool-policy overlays re-baseline next turn"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn model_selector_global_action_sends_persist_global_and_session_action_sends_none() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [{ "id": "openai-api/gpt-5.6-luna", "provider": "OpenAI API" }]
    })));
    a.handle_model_selector_key(&Key::Tab);
    a.handle_model_selector_key(&Key::Tab);
    a.handle_model_selector_key(&Key::Enter);
    let cmds = h.drain_commands().await;
    let set_model: serde_json::Value = cmds
        .iter()
        .filter_map(|c| serde_json::from_str::<serde_json::Value>(c).ok())
        .find(|v| v["type"] == "set_model")
        .expect("set_model sent");
    assert_eq!(set_model["persist"], "global");

    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [{ "id": "openai-api/gpt-5.6-luna", "provider": "OpenAI API" }]
    })));
    a.handle_model_selector_key(&Key::Enter);
    let cmds = h.drain_commands().await;
    let set_model: serde_json::Value = cmds
        .iter()
        .filter_map(|c| serde_json::from_str::<serde_json::Value>(c).ok())
        .find(|v| v["type"] == "set_model")
        .expect("set_model sent");
    assert!(
        set_model.get("persist").is_none(),
        "session-only switches carry no persist: {set_model}"
    );
}

#[tokio::test]
async fn a_refused_pin_is_reported_as_a_failed_switch() {
    let mut h = harness().await;
    h.event(crate::protocol::client::Event::Response {
        id: None,
        command: "set_model".into(),
        success: false,
        data: None,
        error: Some(
            "model not switched: `openai-api/gpt-5.6-luna` could not be recorded as the local default: overlay /repo/.quecto/config.json is not trusted (sha256 ab); review it and run `quecto config trust` first".into(),
        ),
    });
    let text = h.notification_messages().join("\n");
    assert!(text.contains("Model switch failed"), "{text}");
    assert!(text.contains("quecto config trust"), "{text}");
}
