use super::app_render_helpers::strip_ansi;
use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn chat_text(app: &mut App) -> String {
    app.ac_mut()
        .master_session
        .chat
        .render(120)
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn show_session_stats_with_context_updates_footer_flag() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "sessionKey": "cli:foo",
        "totalMessages": 5,
        "tokens": {"input": 10, "output": 20, "cacheRead": 3, "cacheWrite": 2, "total": 30},
        "costMicroUsd": 123400,
        "cacheHitRatio": 3.0 / 15.0,
        "contextTokens": 100,
        "maxContextTokens": 1000
    });
    let a = h.app_mut();
    a.show_session_stats(&data);
    assert!(a.ac().sessions.context_stats_requested);
    let text = chat_text(a);
    assert!(text.contains("Session: cli:foo"));
    assert!(text.contains("Tokens: ↑10 ↓20"));
    assert!(text.contains("Cache: read 3 write 2"));
    assert!(text.contains("Cache hit: 20.0%"));
    assert!(text.contains("Cost: $0.123400"));
    let footer = a.ac_mut().master_session.footer.render(160).join("\n");
    assert!(footer.contains("100/1.0k"), "{footer}");
    assert!(footer.contains("cache 3/2"), "{footer}");
    assert!(footer.contains("hit 20.0%"), "{footer}");
    assert!(footer.contains("cost $0.123400"), "{footer}");
}
#[tokio::test]
async fn show_session_stats_without_context_leaves_flag_false() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessionKey": "cli:bar"});
    let a = h.app_mut();
    a.show_session_stats(&data);
    assert!(!a.ac().sessions.context_stats_requested);
}
#[tokio::test]
async fn update_footer_stats_consumes_positive_cost_without_context() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_footer_stats(&serde_json::json!({ "costMicroUsd": 1_250_000 }));
    assert!(!a.ac().sessions.context_stats_requested);
    let footer = a.ac_mut().master_session.footer.render(120).join("\n");
    assert!(footer.contains("cost $1.250000"), "{footer}");
}

#[tokio::test]
async fn reset_session_clears_chat_and_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.ac_mut()
        .master_session
        .chat
        .add_entry(ChatEntry::User { text: "x".into() });
    a.update_footer_stats(&serde_json::json!({
        "tokens": {"cacheRead": 3, "cacheWrite": 2},
        "cacheHitRatio": 0.2,
        "costMicroUsd": 123_400
    }));
    assert!(
        a.ac_mut()
            .master_session
            .footer
            .render(160)
            .join("\n")
            .contains("cost $0.123400")
    );
    a.ac_mut().sessions.context_stats_requested = true;
    a.reset_session("New session");
    assert_eq!(a.ac().master_session.chat.entry_count(), 0);
    assert!(!a.ac().sessions.context_stats_requested);
    let footer = a.ac_mut().master_session.footer.render(160).join("\n");
    assert!(!footer.contains("cost $"), "{footer}");
    assert!(!footer.contains("cache 3/2"), "{footer}");
    assert!(!footer.contains("hit 20.0%"), "{footer}");
    assert!(!a.notifications.is_empty());
}
