use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn resume_selector_renders_chat_metadata_and_uses_key_for_selection() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "sessions": [{
            "key": "chat-123",
            "title": "Fix the auth bug",
            "name": "Fix the auth bug",
            "messageCount": 12,
            "updatedUnixSecs": 1781980920u64
        }]
    });
    let a = h.app_mut();

    // Empty manifest path so operator workspace sidecars cannot inflate the list.
    let empty_manifest = std::env::temp_dir().join(format!(
        "quecto-resume-selector-empty-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&empty_manifest);
    a.open_resume_selector_at(&data, &empty_manifest);

    let selector = a.ac_mut().sessions.resume_selector.as_mut().unwrap();
    assert_eq!(selector.item_count(), 1);
    let rendered = selector.render_text(80);
    assert!(rendered.contains("Fix the auth bug"));
    assert!(rendered.contains("12 msgs"));
    assert!(
        rendered.contains("2026"),
        "date/time should be present: {rendered}"
    );

    a.handle_resume_selector_key(&Key::Enter);
    let sent = h.drain_commands().await;
    assert!(sent.iter().any(|cmd| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(cmd) else {
            return false;
        };
        value.get("type").and_then(|v| v.as_str()) == Some("resume_session")
            && value.get("session").and_then(|v| v.as_str()) == Some("chat-123")
    }));
}

#[test]
fn resume_picker_focus_cycles_without_toggling_and_activation_exposes_global_preserving_ctrl_g() {
    let client = Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(100, 30);
    let mut app = App::new(term, client);
    let data = serde_json::json!({"sessions": [
        {"key":"local","title":"Local work","scope":{"kind":"scoped","executionLocation":"/work/here","repositoryLabel":"here","isLocal":true}},
        {"key":"other","title":"Other work","scope":{"kind":"scoped","executionLocation":"/work/other","repositoryLabel":"other","isLocal":false}},
        {"key":"legacy","title":"Legacy","scope":{"kind":"legacy_unscoped"}}
    ]});
    let manifest = std::env::temp_dir().join("quecto-d5-empty-manifest.json");
    app.open_resume_selector_at(&data, &manifest);
    let labels = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .unwrap()
        .items_for_tests()
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["Local work"]);

    use super::app_sessions::ResumePickerFocus;
    app.handle_key(Key::Tab);
    assert_eq!(app.ac().sessions.resume_focus, ResumePickerFocus::Query);
    app.handle_key(Key::Tab);
    assert_eq!(app.ac().sessions.resume_focus, ResumePickerFocus::Results);
    app.handle_key(Key::BackTab);
    assert_eq!(app.ac().sessions.resume_focus, ResumePickerFocus::Query);
    app.handle_key(Key::BackTab);
    assert_eq!(app.ac().sessions.resume_focus, ResumePickerFocus::Scope);
    assert_eq!(
        app.ac()
            .sessions
            .resume_selector
            .as_ref()
            .unwrap()
            .items_for_tests()
            .len(),
        1
    );
    app.handle_key(Key::Enter);
    let labels = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .unwrap()
        .items_for_tests()
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["Local work", "Other work", "Legacy"]);
    assert!(app.ac().sessions.resume_selector.is_some());
    app.handle_key(Key::Ctrl('g'));
    assert!(
        app.ac().sessions.resume_selector.is_some(),
        "Ctrl+G remains global jump and must not mutate picker scope"
    );
}

#[test]
fn resume_picker_mouse_scope_and_out_of_bounds_obey_hit_regions() {
    let client = Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(100, 30);
    let mut app = App::new(term, client);
    let data = serde_json::json!({"sessions":[
        {"key":"local","title":"Local","scope":{"kind":"scoped","isLocal":true}},
        {"key":"other","title":"Other","scope":{"kind":"scoped","isLocal":false}}
    ]});
    app.open_resume_selector_at(&data, &std::env::temp_dir().join("d5-mouse.json"));
    app.render();
    let (left, top, width, _) = app.ac().sessions.resume_bounds.unwrap();
    app.handle_key(Key::MouseClick { col: 0, row: 0 });
    assert!(!app.ac().sessions.resume_global, "outside click is a no-op");
    app.handle_key(Key::MouseClick {
        col: left + width - 1,
        row: top,
    });
    assert!(
        app.ac().sessions.resume_global,
        "Global segment click activates scope"
    );
}

#[tokio::test]
async fn cross_folder_selection_uses_typed_decision_and_cancel_sends_nothing() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions":[
        {"key":"local","title":"Local","scope":{"kind":"scoped","executionLocation":"/here","isLocal":true}},
        {"key":"other","title":"Other","scope":{"kind":"scoped","executionLocation":"/other","isLocal":false}}
    ]});
    let a = h.app_mut();
    let manifest = std::env::temp_dir().join("quecto-d5-dialog.json");
    a.open_resume_selector_at(&data, &manifest);
    a.handle_resume_selector_key(&Key::Enter);
    a.handle_resume_selector_key(&Key::Tab);
    a.handle_resume_selector_key(&Key::Tab);
    a.handle_resume_selector_key(&Key::Down);
    a.handle_resume_selector_key(&Key::Enter);
    assert!(a.ac().sessions.resume_decision.is_some());
    a.handle_resume_decision_key(&Key::Enter);
    let sent = h.drain_commands().await;
    assert!(sent.iter().any(
        |line| line.contains("\"type\":\"resume_decision\"") && line.contains("open_original")
    ));
    assert!(
        sent.iter()
            .all(|line| !line.contains("\"type\":\"resume_session\""))
    );
}

#[tokio::test]
async fn cross_folder_dialog_escape_cancels_without_dispatch() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions":[
        {"key":"local","title":"Local","scope":{"kind":"scoped","executionLocation":"/here","isLocal":true}},
        {"key":"other","title":"Other","scope":{"kind":"scoped","executionLocation":"/other","isLocal":false}}
    ]});
    let a = h.app_mut();
    a.open_resume_selector_at(&data, &std::env::temp_dir().join("quecto-d5-cancel.json"));
    a.handle_resume_selector_key(&Key::Enter);
    a.handle_resume_selector_key(&Key::Tab);
    a.handle_resume_selector_key(&Key::Tab);
    a.handle_resume_selector_key(&Key::Down);
    a.handle_resume_selector_key(&Key::Enter);
    assert!(a.ac().sessions.resume_decision.is_some());
    a.handle_resume_decision_key(&Key::Escape);
    assert!(a.ac().sessions.resume_decision.is_none());
    assert!(
        h.drain_commands()
            .await
            .iter()
            .all(|line| !line.contains("resume_decision"))
    );
}

#[tokio::test]
async fn send_new_session_requests_fresh_chat_key() {
    let mut h = harness().await;
    let a = h.app_mut();

    a.send_new_session();

    let sent = h.drain_commands().await;
    assert!(sent.iter().any(|cmd| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(cmd) else {
            return false;
        };
        value.get("type").and_then(|v| v.as_str()) == Some("new_session")
            && value.get("id").is_none()
    }));
}

#[test]
fn format_utc_minutes_formats_known_timestamps() {
    use super::app_methods::format_utc_minutes;
    // 1_700_000_000 = 2023-11-14 22:13:20 UTC (the UTC fallback path).
    assert_eq!(format_utc_minutes(1_700_000_000), "2023-11-14 22:13");
    assert_eq!(format_utc_minutes(0), "1970-01-01 00:00");
}
