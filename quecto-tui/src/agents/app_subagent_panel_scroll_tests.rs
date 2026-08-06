use super::tui_harness::*;
use crate::protocol::client::Event;
use crate::shell::keys::Key;

#[tokio::test]
async fn panel_scroll_and_page_keys_clamp_at_list_edges() {
    let mut h = TuiHarness::sized(80, 8).await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(
        (0..6)
            .map(|i| subagent(&format!("worker-{i:02}"), "running", None))
            .collect(),
    ));

    h.app_mut().handle_key(Key::Tab);
    h.app_mut().handle_key(Key::ScrollUp);
    assert_eq!(
        h.app_mut().panel_highlight_index(),
        0,
        "wheel up at the top of the focused panel must clamp, not wrap to the bottom"
    );

    h.app_mut().handle_key(Key::PageDown);
    h.app_mut().handle_key(Key::PageDown);
    assert_eq!(
        h.app_mut().panel_highlight_index(),
        6,
        "repeated PageDown should clamp to the last panel row"
    );
    h.app_mut().handle_key(Key::ScrollDown);
    assert_eq!(
        h.app_mut().panel_highlight_index(),
        6,
        "wheel down at the bottom of the focused panel must clamp, not wrap to Master"
    );
    h.app_mut().handle_key(Key::PageUp);
    assert_eq!(
        h.app_mut().panel_highlight_index(),
        3,
        "PageUp uses the panel scroll quantum without wrapping"
    );
}

#[tokio::test]
async fn focused_panel_scroll_survives_live_agent_updates() {
    let mut h = TuiHarness::sized(80, 8).await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(
        (0..10)
            .map(|i| subagent(&format!("worker-{i:02}"), "running", None))
            .collect(),
    ));

    h.app_mut().handle_key(Key::Tab);
    h.app_mut().handle_key(Key::PageDown);
    let selected = h.app_mut().panel_highlight_index();
    let before = h.left_panel();
    assert!(
        before.contains("worker-02"),
        "precondition: panel viewport should be scrolled before update:\n{before}"
    );

    h.event(subagents_changed(
        (0..10)
            .map(|i| subagent(&format!("worker-{i:02}"), "idle", None))
            .collect(),
    ));

    assert_eq!(
        h.app_mut().panel_highlight_index(),
        selected,
        "a live status update while panel-focused must not snap the cursor back to the active session"
    );
    let after = h.left_panel();
    assert!(
        after.contains("worker-02"),
        "panel scroll viewport should remain stable across live updates:\n{after}"
    );
}
