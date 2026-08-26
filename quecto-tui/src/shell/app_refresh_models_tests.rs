//! Tests for the TUI `/refresh-models` adapter (epic #1193, slice 4): the
//! slash command drives the harness's one refresh operation and the TUI only
//! renders the reported per-source outcomes — it owns no discovery and keeps
//! no parallel model list.

use super::tui_harness::TuiHarness;

#[tokio::test]
async fn refresh_models_slash_command_sends_the_refresh_operation() {
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_submit("/refresh-models");
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("refresh_models")),
        "/refresh-models must send the refresh_models operation: {cmds:?}"
    );
}

#[tokio::test]
async fn refresh_models_response_renders_per_source_outcomes() {
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_refresh_models(Some(serde_json::json!({
        "outcomes": [
            {"source": "openrouter", "status": "updated", "models": 2},
            {"source": "anthropic-api", "status": "unsupported",
             "reason": "provider does not expose a model listing endpoint"}
        ],
        "generation": 4
    })));
    let text = h.notification_messages().join("\n");
    assert!(text.contains("openrouter: 2 model(s)"), "got: {text}");
    assert!(
        text.contains("anthropic-api: not refreshable"),
        "got: {text}"
    );
}

#[tokio::test]
async fn refresh_models_response_with_no_outcomes_says_nothing_to_refresh() {
    let mut h = TuiHarness::new().await;
    h.app_mut()
        .handle_refresh_models(Some(serde_json::json!({ "outcomes": [] })));
    let text = h.notification_messages().join("\n");
    assert!(text.contains("Nothing to refresh"), "got: {text}");
}
