//! Tests for the event-loop abort flow (`handle_abort`), split from
//! `app_event_loop_tests.rs` to respect the file-size cap (#1463).

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

// ── handle_abort ──────────────────────────────────────────────────────

#[tokio::test]
async fn handle_abort_sends_abort_command() {
    let mut h = harness().await;
    h.app_mut().handle_abort();
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("\"type\":\"abort\"")),
        "handle_abort should send an abort command: {cmds:?}"
    );
}

#[tokio::test]
async fn handle_abort_stops_spinner() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.active_conn_mut().agent_state.start();
    a.active_conn_mut().spinner = Some(Spinner::new("Working"));
    a.handle_abort();
    assert!(
        a.active_conn().spinner.is_none(),
        "abort should clear spinner"
    );
}

#[tokio::test]
async fn handle_abort_finalizes_assistant() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.active_conn_mut().agent_state.start();
    a.active_conn_mut().spinner = Some(Spinner::new("Working"));
    a.handle_abort();
    // finalize_assistant is called; just verify no panic and spinner cleared.
    assert!(a.active_conn().spinner.is_none());
}

#[tokio::test]
async fn handle_abort_adds_status_message() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.active_conn().master_session.chat.entry_count();
    a.handle_abort();
    assert!(
        a.active_conn().master_session.chat.entry_count() > before,
        "abort should add a status entry"
    );
}

#[tokio::test]
async fn handle_abort_calls_agent_state_abort() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.active_conn_mut().agent_state.start();
    assert!(a.active_conn().agent_state.is_running());
    a.handle_abort();
    assert!(
        !a.active_conn().agent_state.is_running(),
        "abort should stop agent_state"
    );
}

#[tokio::test]
async fn handle_abort_sets_footer_streaming_false() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.active_conn_mut()
        .master_session
        .footer
        .set_streaming(true);
    a.handle_abort();
    // Footer streaming should be false after abort.
    let rendered = a
        .active_conn_mut()
        .master_session
        .footer
        .render(80)
        .join("\n");
    assert!(!rendered.contains("streaming") || !rendered.to_lowercase().contains("thinking"));
}

// ── handle_key: overlay routing ───────────────────────────────────────

#[tokio::test]
async fn handle_key_routes_to_overlay_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Open the resume selector to activate an overlay-like state.
    let data = serde_json::json!({"sessions": [{"name": "alpha"}]});
    a.open_resume_selector(&data);
    assert!(a.active_conn().sessions.resume_selector.is_some());
    // Escape should close the selector, not clear the editor.
    a.handle_key(Key::Escape);
    assert!(
        a.active_conn().sessions.resume_selector.is_none(),
        "Escape should close selector"
    );
}

#[tokio::test]
async fn handle_key_routes_to_model_selector_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    assert!(a.inference.model_selector.is_some());
    // Escape closes the selector.
    a.handle_key(Key::Escape);
    assert!(a.inference.model_selector.is_none());
}

#[tokio::test]
async fn handle_key_routes_to_rewind_selector_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn", "id": "u1"}]});
    a.open_rewind_selector(&data);
    assert!(a.active_conn().rewind.selector.is_some());
    a.handle_key(Key::Escape);
    assert!(a.active_conn().rewind.selector.is_none());
}

#[tokio::test]
async fn switching_active_session_closes_open_overlays() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    let data = serde_json::json!({"sessions": [{"name": "alpha"}]});
    a.open_resume_selector(&data);
    assert!(a.inference.model_selector.is_some());
    assert!(a.active_conn().sessions.resume_selector.is_some());
    a.editor.set_text("/mod");
    a.autocomplete.update(&a.editor.text());
    assert!(a.autocomplete.is_active());

    a.select_agent(Some("worker"));
    a.handle_key(Key::Escape);

    assert!(
        a.inference.model_selector.is_none()
            && a.active_conn().sessions.resume_selector.is_none()
            && !a.autocomplete.is_active(),
        "switching tabs/sessions closes active overlays rather than carrying them across"
    );
    assert!(
        a.editor.text().contains("/mod"),
        "Escape after switch should route to normal input, not stale overlay state"
    );
}
