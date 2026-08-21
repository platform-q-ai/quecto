//! `get_state` → durable session key → workspace manifest contract (#1534).

use super::tui_harness::TuiHarness;

/// Cross-crate contract pin: the harness serves `get_state` as the SLIM
/// projection (#1512) — state/effort/model/progress/generation — plus
/// `sessionKey`. The TUI's only way to learn its agent's durable key is that
/// field; without it every workspace manifest saves `session_key: null` and
/// `/resume` restores tabs with no conversations inside. This test feeds the
/// exact slim wire shape to prove the key still round-trips into the
/// manifest snapshot.
#[tokio::test]
async fn slim_get_state_session_key_reaches_the_workspace_manifest() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let slim = serde_json::json!({
        "state": "idle",
        "effort": null,
        "model": "mock-model",
        "progress": {"state": "quiet", "reason": "no tool activity in the last 120 seconds"},
        "generation": 3,
        "sessionKey": "chat-1787000000-deadbeef00000",
    });
    a.handle_response(
        Some("st1".into()),
        "get_state".into(),
        true,
        Some(slim),
        None,
    );
    assert_eq!(
        a.ac().session_key.as_deref(),
        Some("chat-1787000000-deadbeef00000"),
        "slim get_state must teach the TUI its durable session key"
    );
    let ws = a.workspace_id.clone();
    let manifest = a.workspace_manifest_snapshot(&ws);
    assert_eq!(
        manifest.tabs[0].session_key.as_deref(),
        Some("chat-1787000000-deadbeef00000"),
        "the durable key must be persisted for /resume workspace restore"
    );
}
