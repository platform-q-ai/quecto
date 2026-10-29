use super::*;
use crate::components::ansi::{AnsiSegment, ansi_segments};
use crate::components::chat::Chat;
use crate::components::utils::visible_width;
use crate::shell::app::app_events::app_events_test_support::test_app;

fn plain(line: &str) -> String {
    ansi_segments(line)
        .filter_map(|segment| match segment {
            AnsiSegment::Text(text) => Some(text),
            AnsiSegment::Escape(_) => None,
        })
        .collect()
}

fn compact(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn rendered_payload(chat: &mut Chat) -> String {
    let rows = chat.render(80);
    assert!(rows.iter().all(|row| visible_width(row) <= 80));
    compact(&rows.iter().map(|row| plain(row)).collect::<String>())
}

fn long_diagnostic() -> String {
    format!(
        "original-provider-error:{}:original-end\ncorrection-provider-down:{}:appended-end",
        "E".repeat(100),
        "D".repeat(100)
    )
}

async fn check_retained_diagnostic(command: &str, success: bool) {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    app.ac_mut().pending_resume_messages_id = Some("resume-owner".into());
    app.ac_mut().pending_rewind_refresh_id = Some("rewind-owner".into());
    app.ac_mut().pending_attach_backfill_id = Some("attach-owner".into());
    let diagnostic = long_diagnostic();
    app.handle_event(Event::Response {
        id: Some("root-prompt-owner".into()),
        command: command.into(),
        success,
        data: None,
        error: Some(diagnostic.clone()),
    });
    assert!(app.ac().master_session.running);
    assert!(app.ac().agent_state.is_running());
    assert!(app.ac().spinner.is_some());
    assert_eq!(
        app.ac().pending_resume_messages_id.as_deref(),
        Some("resume-owner")
    );
    assert_eq!(
        app.ac().pending_rewind_refresh_id.as_deref(),
        Some("rewind-owner")
    );
    assert_eq!(
        app.ac().pending_attach_backfill_id.as_deref(),
        Some("attach-owner")
    );
    let expected = format!("Error: {diagnostic}");
    assert_eq!(app.notifications.messages(), vec![expected.clone()]);
    assert!(
        app.notifications
            .render(80)
            .iter()
            .map(|row| plain(row))
            .collect::<String>()
            .contains("Error:")
    );

    // Exercise the real transient lifetime, not a second heap-string accessor.
    tokio::time::sleep(std::time::Duration::from_millis(3100)).await;
    assert!(app.notifications.gc());
    assert!(app.notifications.render(80).is_empty());
    let chat = &mut app.ac_mut().master_session.chat;
    let expected = compact(&expected);
    assert_eq!(
        rendered_payload(chat),
        expected,
        "full original and appended diagnostic must render at width80 after toast expiry: {command} success={success}"
    );
    assert_eq!(
        chat.entries()
            .iter()
            .filter(|entry| matches!(entry, ChatEntry::Status { .. }))
            .count(),
        1
    );

    // Move the diagnostic off screen, then recover it via the existing history
    // viewport/scroll path (including evicted render-cache reconstruction).
    chat.set_viewport_height(2);
    for index in 0..20 {
        chat.add_entry(ChatEntry::User {
            text: format!("later-output-{index}"),
        });
    }
    assert!(rendered_payload(chat).contains("later-output-19"));
    let mut history_rows = Vec::new();
    for _ in 0..100 {
        let rows = chat.render(80);
        assert!(rows.iter().all(|row| visible_width(row) <= 80));
        history_rows.push(plain(&rows[0]));
        if chat.is_at_oldest_loaded_history() {
            break;
        }
        chat.scroll_up(1);
    }
    assert!(
        chat.is_at_oldest_loaded_history(),
        "bounded scan must reach retained history"
    );
    assert!(chat.scroll_offset() > 0);
    history_rows.reverse();
    assert!(
        compact(&history_rows.concat()).contains(&expected),
        "full diagnostic must be recoverable by scrolling at width80"
    );
    chat.scroll_to_latest();
    assert!(rendered_payload(chat).contains("later-output-19"));
    assert_eq!(chat.scroll_offset(), 0);
}

#[tokio::test]
async fn run47_diagnostic_presentation_prompt_success() {
    check_retained_diagnostic("prompt", true).await;
}

#[tokio::test]
async fn run47_diagnostic_presentation_prompt_failure() {
    check_retained_diagnostic("prompt", false).await;
}

#[tokio::test]
async fn run47_diagnostic_presentation_follow_up_success() {
    check_retained_diagnostic("follow_up", true).await;
}

#[tokio::test]
async fn run47_diagnostic_presentation_follow_up_failure() {
    check_retained_diagnostic("follow_up", false).await;
}

#[tokio::test]
async fn run47_diagnostic_presentation_clean_controls() {
    for (command, success, error) in [
        ("prompt", true, None),
        ("prompt", false, None),
        ("follow_up", true, None),
        ("follow_up", false, None),
        ("get_state", true, Some("unrelated error")),
    ] {
        let mut app = test_app().await;
        app.handle_event(Event::AgentStart);
        let before = rendered_payload(&mut app.ac_mut().master_session.chat);
        app.handle_event(Event::Response {
            id: Some("clean-owner".into()),
            command: command.into(),
            success,
            data: None,
            error: error.map(str::to_string),
        });
        assert_eq!(
            rendered_payload(&mut app.ac_mut().master_session.chat),
            before
        );
        assert!(app.notifications.render(80).is_empty());
        assert!(app.ac().master_session.running);
        assert!(app.ac().agent_state.is_running());
        assert!(app.ac().spinner.is_some());
    }
}

#[test]
fn run47_diagnostic_presentation_existing_status_presenter() {
    let mut chat = Chat::new();
    let expected = format!("Error: {}", long_diagnostic());
    chat.add_entry(ChatEntry::Status {
        text: expected.clone(),
    });
    assert_eq!(
        rendered_payload(&mut chat),
        compact(&expected),
        "existing status presenter must expose every original/appended character at width80"
    );
}
