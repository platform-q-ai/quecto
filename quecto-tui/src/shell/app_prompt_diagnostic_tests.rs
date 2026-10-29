use super::*;
use crate::shell::app::app_events::app_events_test_support::test_app;

const DIAGNOSTIC: &str =
    "prompt execution failed: original provider error\ncorrection provider down";

fn visible_errors(app: &mut App) -> String {
    let statuses = app
        .ac()
        .master_session
        .chat
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            ChatEntry::Status { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n{statuses}", app.notifications.render(300).join("\n"))
}

fn response(app: &mut App, id: &str, command: &str, success: bool, error: Option<&str>) {
    app.handle_event(Event::Response {
        id: Some(id.into()),
        command: command.into(),
        success,
        data: None,
        error: error.map(str::to_string),
    });
}

async fn check_diagnostic(command: &str, success: bool) {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    app.ac_mut().pending_resume_messages_id = Some("resume-owner".into());
    app.ac_mut().pending_rewind_refresh_id = Some("rewind-owner".into());
    app.ac_mut().pending_attach_backfill_id = Some("attach-owner".into());
    response(
        &mut app,
        "root-prompt-owner",
        command,
        success,
        Some(DIAGNOSTIC),
    );
    assert!(
        app.ac().master_session.running,
        "receipt is not a lifecycle event"
    );
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
    let visible = visible_errors(&mut app);
    assert!(
        visible.contains(DIAGNOSTIC),
        "full original/appended diagnostic discarded for {command} success={success}: {visible:?}"
    );
    assert_eq!(
        visible.matches(DIAGNOSTIC).count(),
        1,
        "diagnostic must appear once"
    );
}

#[tokio::test]
async fn run45_prompt_diagnostic_prompt_success() {
    check_diagnostic("prompt", true).await;
}

#[tokio::test]
async fn run45_prompt_diagnostic_prompt_failure() {
    check_diagnostic("prompt", false).await;
}

#[tokio::test]
async fn run45_prompt_diagnostic_follow_up_success() {
    check_diagnostic("follow_up", true).await;
}

#[tokio::test]
async fn run45_prompt_diagnostic_follow_up_failure() {
    check_diagnostic("follow_up", false).await;
}

#[tokio::test]
async fn run45_prompt_diagnostic_clean_ack_control() {
    for command in ["prompt", "follow_up"] {
        let mut app = test_app().await;
        app.handle_event(Event::AgentStart);
        let before = visible_errors(&mut app);
        response(&mut app, "clean-owner", command, true, None);
        assert_eq!(visible_errors(&mut app), before);
        assert!(app.ac().master_session.running);
        assert!(app.ac().agent_state.is_running());
    }
}

#[tokio::test]
async fn run45_prompt_diagnostic_legacy_error_control() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    app.handle_event(Event::Response {
        id: None,
        command: "agent_error".into(),
        success: false,
        data: None,
        error: Some(DIAGNOSTIC.into()),
    });
    assert!(visible_errors(&mut app).contains(DIAGNOSTIC));
    assert!(!app.ac().master_session.running);
    assert!(!app.ac().agent_state.is_running());
    assert!(app.ac().spinner.is_none());
}

#[tokio::test]
async fn run45_prompt_diagnostic_unrelated_success_control() {
    let mut app = test_app().await;
    let before = visible_errors(&mut app);
    response(
        &mut app,
        "unrelated-owner",
        "get_state",
        true,
        Some(DIAGNOSTIC),
    );
    assert_eq!(
        visible_errors(&mut app),
        before,
        "no generic error interception"
    );
}

#[tokio::test]
async fn run45_prompt_diagnostic_routed_child_control() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    let before = visible_errors(&mut app);
    response(
        &mut app,
        "subagent-state:5:child:request",
        "prompt",
        true,
        Some(DIAGNOSTIC),
    );
    assert_eq!(
        visible_errors(&mut app),
        before,
        "routed child receipt must not reach master"
    );
    assert!(app.ac().master_session.running);
    assert!(app.ac().agent_state.is_running());
}
