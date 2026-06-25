//! Behavioural tests for the sub-agent inspector wiring (#795), driven through
//! the headless render harness and the real key handler.

use super::tui_harness::*;
use crate::infrastructure::client::Event;
use crate::interface::ansi::strip_ansi;
use crate::interface::keys::Key;

async fn with_one_subagent() -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("worker", "running", Some(("active", 1, 3))),
        subagent("other", "running", Some(("active", 2, 3))),
    ]));
    h
}

#[tokio::test]
async fn double_up_opens_inspector_when_editor_empty_and_subagents_present() {
    let mut h = with_one_subagent().await;
    let app = h.app_mut();
    assert!(!app.inspector_open());
    app.handle_key(Key::Up);
    app.handle_key(Key::Up);
    assert!(
        app.inspector_open(),
        "two quick Up presses should open the panel"
    );
}

#[tokio::test]
async fn double_up_does_not_open_while_typing() {
    let mut h = with_one_subagent().await;
    let app = h.app_mut();
    app.handle_key(Key::Char('h'));
    app.handle_key(Key::Up);
    app.handle_key(Key::Up);
    assert!(
        !app.inspector_open(),
        "Up with text in the editor must behave as today (no inspector)"
    );
}

#[tokio::test]
async fn double_up_does_nothing_without_subagents() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let app = h.app_mut();
    app.handle_key(Key::Up);
    app.handle_key(Key::Up);
    assert!(!app.inspector_open(), "no tracked sub-agents → no panel");
}

#[tokio::test]
async fn inspector_claims_input_so_printable_keys_do_not_reach_editor() {
    let mut h = with_one_subagent().await;
    let app = h.app_mut();
    app.handle_key(Key::Up);
    app.handle_key(Key::Up);
    assert!(app.inspector_open());
    app.handle_key(Key::Char('z'));
    // The frame must not show a 'z' echoed into the editor — the inspector
    // consumed it (the editor is hidden while the panel is open anyway).
    let frame = strip_ansi(&app.compose_frame().join("\n"));
    assert!(frame.contains("Sub-agents"), "still in inspector:\n{frame}");
}

#[tokio::test]
async fn esc_steps_focus_then_closes() {
    let mut h = with_one_subagent().await;
    let app = h.app_mut();
    app.handle_key(Key::Up);
    app.handle_key(Key::Up);
    assert!(app.inspector_open());
    app.handle_key(Key::Enter); // focus detail
    app.handle_key(Key::Escape); // detail -> list (still open)
    assert!(
        app.inspector_open(),
        "first Esc returns to the list, not closed"
    );
    app.handle_key(Key::Escape); // list -> close
    assert!(!app.inspector_open(), "second Esc closes the panel");
}

#[tokio::test]
async fn compose_frame_renders_inspector_full_screen_and_is_idempotent() {
    let mut h = with_one_subagent().await;
    let app = h.app_mut();
    app.handle_key(Key::Up);
    app.handle_key(Key::Up);
    let a = app.compose_frame();
    let b = app.compose_frame();
    assert_eq!(a, b, "compose_frame must be render-idempotent (no flash)");
    let plain = strip_ansi(&a.join("\n"));
    assert!(plain.contains("Sub-agents"), "title present:\n{plain}");
    assert!(plain.contains("worker"), "agent listed:\n{plain}");
    assert!(plain.contains("1/3"), "workflow status header:\n{plain}");
}

#[tokio::test]
async fn navigation_fetches_the_newly_highlighted_agent() {
    let mut h = with_one_subagent().await;
    h.app_mut().handle_key(Key::Up);
    h.app_mut().handle_key(Key::Up);
    let _ = h.drain_commands().await; // discard the open-time fetch for "other"
    h.app_mut().handle_key(Key::Down); // highlight "worker"
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("\"agent_id\":\"worker\"")),
        "navigating should fetch the newly highlighted agent: {cmds:?}"
    );
}

#[tokio::test]
async fn opening_requests_an_agent_targeted_tail() {
    let mut h = with_one_subagent().await;
    h.app_mut().handle_key(Key::Up);
    h.app_mut().handle_key(Key::Up);
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("get_messages_tail") && c.contains("\"agent_id\":\"other\"")),
        "opening should poll the selected agent's tail: {cmds:?}"
    );
}

#[test]
fn double_up_window_logic() {
    use super::App;
    let now = tokio::time::Instant::now();
    assert!(!App::double_up_should_open(None, now));
    let recent = now - std::time::Duration::from_millis(100);
    assert!(App::double_up_should_open(Some(recent), now));
    let stale = now - std::time::Duration::from_millis(2000);
    assert!(!App::double_up_should_open(Some(stale), now));
}
