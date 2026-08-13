//! Region-coverage tests for `app_rewind` and `app_response`, split from
//! `app_cov_tests.rs` to stay within the line-count gate.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn chat_text(app: &mut App) -> String {
    let lines = app.master_session.chat.render(120);
    lines
        .iter()
        .map(|l| super::app_methods::strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── app_rewind ───────────────────────────────────────────────────────

/// No-regression guard (#865): the TUI's OWN internal `get_state`/stats polling
/// flows through `handle_response` (Response events with ids like `init`/
/// `stats-footer`), NOT the tool path, so it must never add a chat entry / box.
#[tokio::test]
async fn internal_state_polling_response_renders_no_box() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.master_session.chat.entry_count();
    a.handle_event(Event::Response {
        id: Some("init".into()),
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({
            "model": "m",
            "sessionKey": "cli:default",
            "workflow": {"mode":"complete","progress":{"done":5,"total":5,"percent":100}}
        })),
        error: None,
    });
    a.handle_event(Event::Response {
        id: Some("stats-footer".into()),
        command: "get_session_stats".into(),
        success: true,
        data: Some(serde_json::json!({"cost": 0.0})),
        error: None,
    });
    assert_eq!(
        a.master_session.chat.entry_count(),
        before,
        "internal get_state/stats polling must not render a chat box (#865)"
    );
}

#[tokio::test]
async fn rewind_preview_truncates_and_sanitizes() {
    let preview = super::app_rewind::rewind_preview("safe \u{1b}[31mvalue");
    assert!(!preview.contains('\u{1b}'));
    assert!(preview.contains("safe"));
}

#[tokio::test]
async fn idle_escape_first_press_arms_notification() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_idle_escape_for_rewind();
    assert!(a.rewind.last_idle_escape.is_some());
    assert!(a.rewind.pending_open_id.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn idle_escape_double_press_requests_messages() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_idle_escape_for_rewind();
    a.handle_idle_escape_for_rewind();
    assert!(a.rewind.last_idle_escape.is_none());
    assert!(a.rewind.pending_open_id.is_some());
}

#[tokio::test]
async fn open_rewind_selector_no_messages_key_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_rewind_selector(&serde_json::json!({}));
    assert!(a.rewind.selector.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn open_rewind_selector_builds_turns_in_reverse() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "messages": [
            {"role": "user", "content": "one", "id": "u1"},
            {"role": "assistant", "content": "ans", "id": "a1"},
            {"role": "user", "content": "two", "id": "u2"}
        ]
    });
    {
        let a = h.app_mut();
        a.open_rewind_selector(&data);
        assert_eq!(a.rewind.selector.as_ref().unwrap().item_count(), 2);
        let frame = a.compose_frame().join("\n");
        assert!(
            frame.contains("Previous turn: two"),
            "the newest user turn must be labelled as the previous turn; frame={frame}"
        );
        assert!(
            frame.contains("2 turns ago: one"),
            "older user turns must keep their relative-turn labels; frame={frame}"
        );
        a.handle_rewind_selector_key(&Key::Enter);
    }

    let commands = h.drain_commands().await;
    let load = commands
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message")).then_some(cmd)
        })
        .expect("a get_message command must be sent");
    assert_eq!(
        load.get("messageId").and_then(|v| v.as_str()),
        Some("u2"),
        "the newest user turn must be selected first"
    );
}

#[tokio::test]
async fn open_rewind_selector_no_user_turns_notifies() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "assistant", "content": "x"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    assert!(a.rewind.selector.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn rewind_selector_skips_idless_user_turns() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [
        {"role": "user", "content": "idless newest"},
        {"role": "user", "content": "stable older", "id": "stable-u1"}
    ]});
    {
        let a = h.app_mut();
        a.open_rewind_selector(&data);
        assert_eq!(a.rewind.selector.as_ref().unwrap().item_count(), 1);
        let frame = a.compose_frame().join("\n");
        assert!(
            frame.contains("Previous turn: stable older"),
            "the remaining stable-id user turn should still be selectable; frame={frame}"
        );
        assert!(
            !frame.contains("idless newest"),
            "id-less user turns must not be selectable rewind targets; frame={frame}"
        );
        a.handle_rewind_selector_key(&Key::Enter);
    }

    let commands = h.drain_commands().await;
    let load = commands
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message")).then_some(cmd)
        })
        .expect("a get_message command must be sent");
    assert_eq!(
        load.get("messageId").and_then(|v| v.as_str()),
        Some("stable-u1"),
        "only the stable-id user turn may be selected"
    );
}

#[tokio::test]
async fn rewind_selector_enter_requests_rewind() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn", "id": "u1"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Enter);
    assert!(a.rewind.selector.is_none());
    assert!(a.rewind.pending_load_id.is_some());
    assert_eq!(a.rewind.pending_apply_message_id.as_deref(), Some("u1"));
}

#[tokio::test]
async fn rewind_selector_enter_sends_stable_message_id_not_page_local_index() {
    // #1061 blocker: with paged history the selector holds only a bounded
    // window, so it must target the message's STABLE id — never a page-local
    // array index that the server would misapply to the full conversation.
    let mut h = harness().await;
    {
        let a = h.app_mut();
        let data = serde_json::json!({"messages": [
            {"role": "user", "content": "turn", "id": "msg-42"}
        ]});
        a.open_rewind_selector(&data);
        a.handle_rewind_selector_key(&Key::Enter);
    }
    let commands = h.drain_commands().await;
    let rewind = commands
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message")).then_some(cmd)
        })
        .expect("a get_message command must be sent");
    assert_eq!(
        rewind.get("messageId").and_then(|v| v.as_str()),
        Some("msg-42")
    );
    assert!(
        rewind.get("messageIndex").is_none(),
        "must not send a page-local index; command={rewind}"
    );
    assert_eq!(rewind.get("offset").and_then(|v| v.as_u64()), Some(0));
    assert_eq!(
        rewind.get("limit").and_then(|v| v.as_u64()),
        Some(super::app_paged_history::GET_MESSAGE_PAGE_BYTES as u64)
    );
}

#[tokio::test]
async fn rewind_selector_escape_cancels() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Escape);
    assert!(a.rewind.selector.is_none());
    assert!(a.rewind.pending_apply_id.is_none());
}

#[tokio::test]
async fn rewind_selector_pending_keeps_open() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "messages": [
            {"role": "user", "content": "a", "id": "u1"},
            {"role": "user", "content": "b", "id": "u2"}
        ]
    });
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Down);
    assert!(a.rewind.selector.is_some());
}

#[tokio::test]
async fn rewind_request_ids_are_monotonically_increasing() {
    let mut h = harness().await;
    let a = h.app_mut();
    let seq_before = a.rewind.request_seq;
    // First double-escape generates an open request.
    a.handle_idle_escape_for_rewind();
    a.handle_idle_escape_for_rewind();
    let open_id = a.rewind.pending_open_id.as_ref().unwrap().clone();
    assert!(open_id.contains("rewind-open-"));
    let seq_after_open = a.rewind.request_seq;
    assert_eq!(seq_after_open, seq_before.wrapping_add(1));
    // Now simulate a rewind apply (open selector, press Enter).
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn", "id": "u1"}]});
    a.rewind.pending_open_id = None; // simulate response clearing it
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Enter);
    let load_id = a.rewind.pending_load_id.as_ref().unwrap().clone();
    assert!(load_id.contains("rewind-load-"));
    let seq_after_load = a.rewind.request_seq;
    assert_eq!(seq_after_load, seq_before.wrapping_add(2));
    let message = serde_json::json!({"role": "user", "content": "turn", "id": "u1"});
    respond(a, Some(&load_id), "get_message", true, Some(message), None);
    let apply_id = a.rewind.pending_apply_id.as_ref().unwrap().clone();
    assert!(apply_id.contains("rewind-to-"));
    let seq_after_apply = a.rewind.request_seq;
    assert_eq!(seq_after_apply, seq_before.wrapping_add(3));
}

#[tokio::test]
async fn double_escape_outside_window_does_not_open_selector() {
    let mut h = harness().await;
    let a = h.app_mut();
    // First Escape arms.
    a.handle_idle_escape_for_rewind();
    assert!(a.rewind.last_idle_escape.is_some());
    // Simulate passage of time beyond the window by clearing last_idle_escape.
    a.rewind.last_idle_escape = None;
    // Second Escape should arm again (not open selector).
    a.handle_idle_escape_for_rewind();
    assert!(
        a.rewind.last_idle_escape.is_some(),
        "should arm again after window expired"
    );
    assert!(
        a.rewind.pending_open_id.is_none(),
        "should not open selector"
    );
}

// ── app_response: handle_response dispatch ───────────────────────────

fn respond(
    app: &mut App,
    id: Option<&str>,
    command: &str,
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<&str>,
) {
    if command == "get_messages" {
        match id {
            Some("attach-backfill") => app.test_arm_attach_backfill("attach-backfill"),
            Some("resume-messages") => app.test_arm_resume_messages("resume-messages"),
            Some("rewind-refresh") => app.test_arm_rewind_refresh("rewind-refresh"),
            _ => {}
        }
    }
    app.handle_response(
        id.map(String::from),
        command.to_string(),
        success,
        data,
        error.map(String::from),
    );
}

#[tokio::test]
async fn response_get_state_populates_model_and_agent_id() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "model": "anthropic/claude-opus-4-5",
        "maxContextTokens": 200000,
        "sessionKey": "cli:worker",
        "workflow": {
            "progress": {"done": 1, "total": 3},
            "activeIssue": {"number": 4, "title": "t"},
            "automation": {"autoContinue": true, "completionNudge": true}
        }
    });
    let a = h.app_mut();
    respond(a, None, "get_state", true, Some(data), None);
    assert_eq!(
        a.inference.current_model.as_deref(),
        Some("anthropic/claude-opus-4-5")
    );
    assert_eq!(a.conn.connected_agent_id.as_deref(), Some("worker"));
    assert!(a.workflow.auto_continue);
    assert!(a.workflow.completion_nudge);
    assert!(a.sessions.context_stats_requested);
}

#[tokio::test]
async fn response_get_state_default_session_clears_agent_id() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessionKey": "cli:default"});
    let a = h.app_mut();
    respond(a, None, "get_state", true, Some(data), None);
    assert!(a.conn.connected_agent_id.is_none());
}

#[tokio::test]
async fn response_get_state_no_data_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(a, None, "get_state", true, None, None);
    assert!(a.inference.current_model.is_none());
}

#[tokio::test]
async fn response_set_model_success_and_failure() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(a, None, "set_model", true, None, None);
    assert!(!a.notifications.is_empty());
    respond(a, None, "set_model", false, None, Some("boom"));
}

#[tokio::test]
async fn response_set_workflow_automation_success_and_failure() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"autoContinue": true, "completionNudge": false});
    respond(a, None, "set_workflow_automation", true, Some(data), None);
    assert!(a.workflow.auto_continue);
    assert!(!a.notifications.is_empty());
    respond(
        a,
        None,
        "set_workflow_automation",
        false,
        None,
        Some("nope"),
    );
}

#[tokio::test]
async fn response_get_session_stats_renders() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"sessionKey": "cli:s", "totalMessages": 1});
    respond(a, None, "get_session_stats", true, Some(data), None);
    assert!(chat_text(a).contains("Session: cli:s"));
}

#[tokio::test]
async fn response_list_sessions_success_and_failure() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"sessions": [{"name": "alpha"}]});
    respond(a, None, "list_sessions", true, Some(data), None);
    assert!(a.sessions.resume_selector.is_some());
    respond(a, None, "list_sessions", false, None, Some("err"));
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn response_resume_session_success_and_failure() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"session": "alpha"});
    respond(a, None, "resume_session", true, Some(data), None);
    assert!(!a.notifications.is_empty());
    respond(a, None, "resume_session", false, None, Some("err"));
}

#[tokio::test]
async fn response_get_messages_opens_rewind_when_id_matches() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.pending_open_id = Some("rid".into());
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn", "id": "u1"}]});
    respond(a, Some("rid"), "get_messages", true, Some(data), None);
    assert!(a.rewind.selector.is_some());
    assert!(a.rewind.pending_open_id.is_none());
}

#[tokio::test]
async fn response_get_messages_replaces_chat_when_no_match() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
    respond(a, Some("other"), "get_messages", true, Some(data), None);
    let text = chat_text(a);
    assert!(text.contains("hi"));
    assert!(!text.contains("Session resumed"));
}

#[tokio::test]
async fn response_rewind_to_success_clears_pending_and_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.pending_apply_id = Some("rt".into());
    respond(a, Some("rt"), "rewind_to", true, None, None);
    assert!(a.rewind.pending_apply_id.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn response_get_message_for_rewind_sends_rewind_with_full_text() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("draft");
    a.rewind.pending_load_id = Some("load".into());
    a.rewind.pending_apply_message_id = Some("u1".into());
    let data = serde_json::json!({
        "id": "u1",
        "role": "user",
        "content": "full prompt body",
        "contentLength": "full prompt body".len(),
        "hasMoreContent": false,
        "offset": 0
    });
    respond(a, Some("load"), "get_message", true, Some(data), None);
    assert!(a.rewind.pending_load_id.is_none());
    assert!(a.rewind.pending_apply_message_id.is_none());
    assert!(a.rewind.pending_apply_id.is_some());
    assert_eq!(
        a.rewind.pending_apply_text.as_deref(),
        Some("full prompt body")
    );
    assert_eq!(
        a.rewind.pending_apply_editor_baseline.as_deref(),
        Some("draft")
    );
}

#[tokio::test]
async fn response_rewind_to_success_moves_selected_text_into_editor() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("draft");
    a.rewind.pending_apply_id = Some("rt".into());
    a.rewind.pending_apply_editor_baseline = Some("draft".into());
    a.rewind.pending_apply_text = Some("original prompt\nsecond line".into());
    respond(a, Some("rt"), "rewind_to", true, None, None);
    assert_eq!(a.editor.text(), "original prompt\nsecond line");
    assert!(a.rewind.pending_apply_text.is_none());
}

#[tokio::test]
async fn response_rewind_to_success_keeps_newer_editor_draft() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("newer draft");
    a.rewind.pending_apply_id = Some("rt".into());
    a.rewind.pending_apply_editor_baseline = Some("draft at send".into());
    a.rewind.pending_apply_text = Some("original prompt".into());
    respond(a, Some("rt"), "rewind_to", true, None, None);
    assert_eq!(a.editor.text(), "newer draft");
    assert!(a.rewind.pending_apply_text.is_none());
}

#[tokio::test]
async fn response_rewind_to_failure_clears_pending_and_notifies_error() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.pending_apply_id = Some("rt".into());
    a.rewind.pending_apply_editor_baseline = Some("".into());
    a.rewind.pending_apply_text = Some("keep out of editor".into());
    respond(a, Some("rt"), "rewind_to", false, None, Some("bad"));
    assert!(a.rewind.pending_apply_id.is_none());
    assert!(a.rewind.pending_apply_editor_baseline.is_none());
    assert!(a.rewind.pending_apply_text.is_none());
    assert_eq!(a.editor.text(), "");
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn response_rewind_to_unmatched_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(a, Some("zzz"), "rewind_to", true, None, None);
    assert!(a.notifications.is_empty());
}

#[tokio::test]
async fn response_unknown_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.master_session.chat.entry_count();
    respond(a, None, "totally_unknown_command", true, None, None);
    // An unknown response is not surfaced: no chat entries, no notifications.
    assert_eq!(
        a.master_session.chat.entry_count(),
        before,
        "noop responses must not add chat entries"
    );
    assert!(
        a.notifications.is_empty(),
        "noop responses must not raise notifications"
    );
}

#[tokio::test]
async fn response_clear_history_signals_workflow_retained() {
    // #897 AC1: clearing history must visibly distinguish the cleared
    // conversation from the workflow engine state, which is retained by design.
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.master_session.chat.entry_count();
    respond(a, None, "clear_history", true, None, None);
    assert_eq!(
        a.master_session.chat.entry_count(),
        before,
        "clear_history must not add chat entries"
    );
    let text = a
        .notifications
        .render(120)
        .iter()
        .map(|line| super::app_methods::strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(
        text.contains("history cleared"),
        "must signal that history was cleared: {text}"
    );
    assert!(
        text.contains("workflow retained"),
        "must signal that the workflow was retained: {text}"
    );
}

#[tokio::test]
async fn response_get_subagents_updates_bar() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({
        "subagents": [
            {"agentId": "w1", "status": "running", "pid": 0}
        ]
    });
    respond(a, None, "get_subagents", true, Some(data), None);
    assert!(!a.subagents.tracked.is_empty());
}

#[tokio::test]
async fn response_agent_error_appends_status_and_resets() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.conn.spinner = Some(super::Spinner::new("x"));
    respond(a, None, "agent_error", false, None, Some("kaboom"));
    assert!(chat_text(a).contains("kaboom"));
    assert!(a.conn.spinner.is_none());
}

#[tokio::test]
async fn response_agent_error_without_message_uses_default() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(a, None, "agent_error", false, None, None);
    assert!(chat_text(a).contains("unknown error"));
}

fn history_page(messages: &[(&str, &str)], before: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "messages": messages
            .iter()
            .map(|(id, content)| serde_json::json!({
                "id": id, "role": "user", "content": content,
            }))
            .collect::<Vec<_>>(),
        "before": before,
        "hasMoreBefore": before.is_some(),
    })
}

#[tokio::test]
async fn rewind_refresh_replaces_transcript_and_resets_paging_state() {
    let mut h = harness().await;
    // Long attached history: the paging cursor refers to the PRE-rewind
    // conversation (#1061 review — it must not survive the rewind).
    respond(
        h.app_mut(),
        None,
        "get_messages",
        true,
        Some(history_page(&[("m9", "pre-rewind newest")], Some("m9"))),
        None,
    );
    let _ = h.drain_commands().await;

    h.app_mut().rewind.pending_apply_id = Some("rw".into());
    respond(h.app_mut(), Some("rw"), "rewind_to", true, None, None);
    let refresh_id = h
        .app_mut()
        .test_pending_rewind_refresh_id()
        .expect("rewind_to mints a refresh id")
        .to_string();
    respond(
        h.app_mut(),
        Some(&refresh_id),
        "get_messages",
        true,
        Some(history_page(&[("m2", "kept turn")], Some("m2"))),
        None,
    );

    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("kept turn"),
        "refreshed page renders: {frame}"
    );
    assert!(
        !frame.contains("pre-rewind newest"),
        "rewind must replace the pre-rewind transcript, not prepend into it:\n{frame}"
    );
    assert!(
        frame.contains("Conversation rewound"),
        "rewind refresh should announce itself, not claim a resume:\n{frame}"
    );

    // Scroll-back must page with the POST-rewind cursor, never the stale one
    // (which the server would reject as "history cursor not found" forever).
    let _ = h.drain_commands().await;
    {
        let chat = h.app_mut().active_chat_mut();
        chat.set_viewport_height(1);
        let _ = chat.render(120);
    }
    h.app_mut().handle_key(Key::PageUp);
    let commands: Vec<serde_json::Value> = h
        .drain_commands()
        .await
        .into_iter()
        .filter_map(|line| serde_json::from_str(&line).ok())
        .filter(|cmd: &serde_json::Value| {
            cmd.get("type").and_then(|v| v.as_str()) == Some("get_messages")
        })
        .collect();
    assert!(
        commands
            .iter()
            .any(|cmd| cmd.get("before").and_then(|v| v.as_str()) == Some("m2")),
        "post-rewind paging must use the refreshed cursor; commands={commands:?}"
    );
    assert!(
        !commands
            .iter()
            .any(|cmd| cmd.get("before").and_then(|v| v.as_str()) == Some("m9")),
        "the pre-rewind cursor must not survive the rewind; commands={commands:?}"
    );
}

#[tokio::test]
async fn rewind_success_clears_failed_stub_recall_state() {
    // A recall marked permanently failed belongs to the PRE-rewind
    // conversation; after a rewind the same stub id may be recallable again
    // (#1061 review — clear_message_recovery must reset stub-recall state).
    let mut h = harness().await;
    let a = h.app_mut();
    a.conn
        .failed_stub_recalls
        .insert((None, "stub-1".to_string()));
    a.rewind.pending_apply_id = Some("rw".into());
    respond(a, Some("rw"), "rewind_to", true, None, None);
    assert!(
        a.conn.failed_stub_recalls.is_empty(),
        "rewind must reset failed stub-recall markers"
    );
    assert!(a.conn.pending_stub_recall.is_empty());
}
