use super::tui_harness::*;
use crate::protocol::client::Event;
use crate::shell::keys::Key;

fn selected_panel_label(panel: &str) -> Option<String> {
    panel
        .lines()
        .find(|line| line.contains('▌'))
        .map(str::to_string)
}

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

#[tokio::test]
async fn focused_panel_selection_tracks_agent_identity_across_roster_reorder() {
    let mut h = TuiHarness::sized(80, 8).await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("alpha", "running", None),
        subagent("bravo", "running", None),
        subagent("charlie", "running", None),
    ]));

    h.press(Key::Tab);
    h.press(Key::Down);
    h.press(Key::Down);
    assert!(
        selected_panel_label(&h.full_frame()).is_some_and(|line| line.contains("bravo")),
        "precondition: focused panel cursor should be on bravo before reorder:\n{}",
        h.left_panel()
    );

    h.event(subagents_changed(vec![
        subagent("aardvark", "running", None),
        subagent("alpha", "running", None),
        subagent("bravo", "idle", None),
        subagent("charlie", "running", None),
    ]));

    assert!(
        selected_panel_label(&h.full_frame()).is_some_and(|line| line.contains("bravo")),
        "focused panel cursor must follow the same agent identity across roster reorder:\n{}",
        h.left_panel()
    );
    h.press(Key::Enter);
    assert_eq!(
        h.app_mut().active_agent_id(),
        Some("bravo"),
        "Enter must commit the identity the user highlighted before the live reorder"
    );
}

#[tokio::test]
async fn focused_panel_environment_highlight_is_not_restored_by_live_sync() {
    let mut h = TuiHarness::sized(80, 8).await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("alpha", "running", None),
        subagent("bravo", "running", None),
        subagent("charlie", "running", None),
    ]));

    h.press(Key::Tab);
    h.press(Key::Down);
    h.press(Key::Down);
    let selected = h.app_mut().panel_highlight_index();
    assert!(
        selected_panel_label(&h.full_frame()).is_some_and(|line| line.contains("bravo")),
        "precondition: focused panel cursor should be on bravo:\n{}",
        h.left_panel()
    );

    h.app_mut().subagents.selected_environment = Some("stale-env".to_string());
    h.event(subagents_changed(vec![
        subagent("alpha", "idle", None),
        subagent("bravo", "idle", None),
        subagent("charlie", "idle", None),
    ]));

    assert_eq!(
        h.app_mut().panel_highlight_index(),
        selected,
        "Focus::Panel must prevent stale committed environment sync from snapping to active/master"
    );
    assert!(
        selected_panel_label(&h.full_frame()).is_some_and(|line| line.contains("bravo")),
        "focused panel cursor should remain on bravo after stale env clears:\n{}",
        h.left_panel()
    );
}
