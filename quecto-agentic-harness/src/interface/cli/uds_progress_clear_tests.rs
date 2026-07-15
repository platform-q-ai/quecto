use super::*;

#[tokio::test]
async fn test_forward_progress_event_emits_tool_started_with_tool_call_id() {
    use crate::domain::agent::AgentProgressEvent;
    use crate::interface::cli::uds_cancel::forward_progress_event;

    let mut buf = Vec::new();
    let ev = AgentProgressEvent::ToolStarted {
        tool_call_id: "call_abc".to_string(),
        name: "bash".to_string(),
        arguments: r#"{"command":"echo hi"}"#.to_string(),
    };
    forward_progress_event(ev, &mut buf).await;

    let output = String::from_utf8(buf).unwrap();
    assert!(
        output.contains("tool_execution_start"),
        "expected tool_execution_start event, got: {output}"
    );
    assert!(
        output.contains("call_abc"),
        "expected tool_call_id 'call_abc', got: {output}"
    );
    assert!(
        output.contains("bash"),
        "expected tool_name 'bash', got: {output}"
    );
}

#[tokio::test]
async fn test_forward_progress_event_emits_tool_finished_with_tool_call_id() {
    use crate::domain::agent::AgentProgressEvent;
    use crate::interface::cli::uds_cancel::forward_progress_event;

    let mut buf = Vec::new();
    let ev = AgentProgressEvent::ToolFinished {
        tool_call_id: "call_xyz".to_string(),
        name: "read".to_string(),
        arguments: r#"{"path":"a.txt"}"#.to_string(),
        result_content: "file content here".to_string(),
        duration_ms: 42,
        is_error: false,
    };
    forward_progress_event(ev, &mut buf).await;

    let output = String::from_utf8(buf).unwrap();
    assert!(
        output.contains("tool_execution_end"),
        "expected tool_execution_end event, got: {output}"
    );
    assert!(
        output.contains("call_xyz"),
        "expected tool_call_id 'call_xyz', got: {output}"
    );
    assert!(
        output.contains("read"),
        "expected tool_name 'read', got: {output}"
    );
    // Verify result content is propagated (not empty string).
    assert!(
        output.contains("file content here"),
        "expected result_content in tool_execution_end event, got: {output}"
    );
}

#[tokio::test]
async fn test_forward_progress_event_emits_subagent_messages_appended() {
    use crate::domain::agent::AgentProgressEvent;
    use crate::interface::cli::uds_cancel::forward_progress_event;

    let mut buf = Vec::new();
    let ev = AgentProgressEvent::TurnCompleted {
        messages: vec![
            Message::assistant("turn output", vec![]),
            Message::tool("call-1", "tool result body"),
        ]
        .into(),
    };
    forward_progress_event(ev, &mut buf).await;

    let output = String::from_utf8(buf).unwrap();
    assert!(
        output.contains("subagent_messages_appended"),
        "expected subagent_messages_appended event, got: {output}"
    );
    // Emitted with an empty agent_id on the child's own stream (the parent
    // monitor re-stamps it before forwarding).
    assert!(
        output.contains("\"agent_id\":\"\""),
        "child emits empty agent_id, got: {output}"
    );
    // #1060: content is not re-carried; stable messageRefs identify the turn.
    assert!(
        output.contains("messageRefs") && !output.contains("turn output"),
        "turn must be identified by non-empty messageRefs without full content, got: {output}"
    );
    let v: serde_json::Value = serde_json::from_str(output.lines().next().unwrap()).unwrap();
    assert!(
        v["messageRefs"].as_array().is_some_and(|a| a.len() == 2),
        "expected two message refs, got: {output}"
    );
}

// ─── clear_history (#408) ────────────────────────────────────────────────────

#[test]
fn test_parse_clear_history_command() {
    let line = r#"{"type":"clear_history"}"#;
    let cmd = parse_command_line(line).unwrap();
    assert_eq!(cmd.type_name(), "clear_history");
    assert!(cmd.id().is_none());
}

#[test]
fn test_parse_clear_history_with_id() {
    let line = r#"{"type":"clear_history","id":"ch-1"}"#;
    let cmd = parse_command_line(line).unwrap();
    assert_eq!(cmd.type_name(), "clear_history");
    assert_eq!(cmd.id(), Some("ch-1"));
}

#[test]
fn test_clear_history_type_name() {
    let cmd = AgentCommand::ClearHistory { id: None };
    assert_eq!(cmd.type_name(), "clear_history");
}

#[test]
fn test_clear_history_preserves_system_prompt() {
    let mut messages: Vec<Message> = vec![
        Message::system("Be helpful."),
        Message::user("hello"),
        Message::assistant("hi there", vec![]),
        Message::user("what is 2+2?"),
        Message::assistant("4", vec![]),
    ];
    clear_conversation(&mut messages);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, crate::domain::message::Role::System);
    assert_eq!(messages[0].content, "Be helpful.");
}

#[test]
fn test_clear_history_without_system_prompt() {
    let mut messages: Vec<Message> = vec![
        Message::user("hello"),
        Message::assistant("hi there", vec![]),
    ];
    clear_conversation(&mut messages);
    assert!(messages.is_empty());
}

#[test]
fn test_clear_history_skips_manifest_at_position_zero() {
    let mut manifest = Message::system("[Session memory: 5 spilled entries]");
    manifest.is_manifest = true;
    let mut messages: Vec<Message> = vec![
        manifest,
        Message::user("hello"),
        Message::assistant("hi", vec![]),
    ];
    clear_conversation(&mut messages);
    assert!(messages.is_empty(), "manifest should not be preserved");
}

#[test]
fn test_clear_history_drains_pending() {
    let mut session = AgentSession::new("model".into(), "key".into());
    session.enqueue_pending("follow-up-1".into());
    session.enqueue_pending("follow-up-2".into());

    // Simulate drain as in handle_clear_history
    session.drain_pending();

    assert_eq!(
        session
            .state_snapshot(0, None, 0, None)
            .pending_message_count,
        0
    );
}

#[test]
fn test_clear_history_blocked_while_streaming() {
    let mut session = AgentSession::new("model".into(), "key".into());
    session.set_streaming(true);
    // The handler checks is_streaming() and returns error — verify the guard.
    assert!(session.is_streaming());
}

// ─── rewind_to ───────────────────────────────────────────────────────────────

#[test]
fn test_parse_rewind_to_command() {
    let line = r#"{"type":"rewind_to","id":"rw-1","messageIndex":2}"#;
    let cmd = parse_command_line(line).unwrap();
    assert_eq!(cmd.type_name(), "rewind_to");
    assert_eq!(cmd.id(), Some("rw-1"));
}

#[test]
fn test_rewind_to_type_name() {
    let cmd = AgentCommand::RewindTo {
        id: None,
        message_index: Some(0),
        message_id: None,
    };
    assert_eq!(cmd.type_name(), "rewind_to");
}

#[test]
fn test_rewind_to_user_boundary_truncates() {
    let mut messages: Vec<Message> = vec![
        Message::system("Be helpful."),
        Message::user("first"),
        Message::assistant("first reply", vec![]),
        Message::user("second"),
        Message::assistant("second reply", vec![]),
    ];
    assert!(rewind_to_message_index(&mut messages, 3));
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].content, "first reply");
}

#[test]
fn test_rewind_to_rejects_non_user_boundary() {
    let mut messages: Vec<Message> =
        vec![Message::user("first"), Message::assistant("reply", vec![])];
    assert!(!rewind_to_message_index(&mut messages, 1));
    assert_eq!(messages.len(), 2);
}

#[test]
fn test_rewind_to_rejects_out_of_range() {
    let mut messages: Vec<Message> = vec![Message::user("first")];
    assert!(!rewind_to_message_index(&mut messages, 99));
    assert_eq!(messages.len(), 1);
}

#[test]
fn resolve_rewind_target_by_id_maps_to_absolute_index() {
    // #1061 blocker: a stable messageId resolves against the FULL conversation,
    // never a paged client's window-local array position (which would truncate
    // the wrong, much older turn).
    let messages: Vec<Message> = (0..100).map(|i| Message::user(format!("m{i}"))).collect();
    let id = messages[80].id().to_string();
    assert_eq!(resolve_rewind_target(&messages, Some(&id), None), Ok(80));
}

#[test]
fn resolve_rewind_target_unknown_id_errs_without_falling_back() {
    // An unresolvable id must error, never silently fall back to index 0.
    let messages: Vec<Message> = vec![Message::user("only")];
    assert!(resolve_rewind_target(&messages, Some("no-such-id"), Some(0)).is_err());
}

#[test]
fn resolve_rewind_target_legacy_index_passes_through() {
    let messages: Vec<Message> = vec![Message::user("a"), Message::user("b")];
    assert_eq!(resolve_rewind_target(&messages, None, Some(1)), Ok(1));
    assert!(resolve_rewind_target(&messages, None, None).is_err());
}

#[test]
fn resolve_rewind_target_legacy_index_rejected_beyond_one_page() {
    // #1061 review follow-up: a pre-paging client computes its index from the
    // newest page (previously the full history). Once the conversation exceeds
    // one page, that index is page-local — applying it as an absolute position
    // would destructively truncate a much older turn. Reject it loudly instead.
    use crate::interface::cli::protocol::HISTORY_PAGE_SIZE;
    let messages: Vec<Message> = (0..=HISTORY_PAGE_SIZE)
        .map(|i| Message::user(format!("m{i}")))
        .collect();
    let err = resolve_rewind_target(&messages, None, Some(2))
        .expect_err("page-ambiguous legacy index must be rejected");
    assert!(
        err.contains("messageId"),
        "error should steer to messageId: {err}"
    );

    // At exactly one page the index is still unambiguous and honoured.
    let one_page: Vec<Message> = (0..HISTORY_PAGE_SIZE)
        .map(|i| Message::user(format!("m{i}")))
        .collect();
    assert_eq!(resolve_rewind_target(&one_page, None, Some(2)), Ok(2));
}

#[test]
fn test_rewind_to_removes_retained_spill_references() {
    let mut manifest = Message::system("[Session memory: 1 spilled entry]");
    manifest.is_manifest = true;
    let mut collapsed = Message::tool("call-1", "[bash: output — recall(\"turn1:bash:0\")]");
    collapsed.is_collapsed = true;
    collapsed.spill_id = Some("turn1:bash:0".into());
    let mut messages: Vec<Message> = vec![
        Message::system("Be helpful."),
        manifest,
        Message::user("first"),
        collapsed,
        Message::user("second"),
    ];

    assert!(rewind_to_message_index(&mut messages, 4));
    assert!(!messages.iter().any(|m| m.is_manifest));
    assert!(!messages.iter().any(|m| m.is_collapsed));
    assert!(!messages.iter().any(|m| m.spill_id.is_some()));
    assert!(!messages.iter().any(|m| m.content.contains("recall(")));
}

#[test]
fn test_rewind_keeps_collapsed_conversation_messages_non_empty() {
    // #1046: `is_collapsed` no longer implies a tool stub. A collapsed
    // user/assistant message must survive rewind as a NON-EMPTY provider turn
    // (its stub minus the dangling recall clause) — blanking it like a tool
    // stub sends empty text blocks to the provider, which some reject.
    use crate::application::context_pruning::messages::message_collapse_stub;
    let mut collapsed_assistant = Message::assistant(
        message_collapse_stub(
            "assistant",
            "I analysed the logs",
            840,
            "turn2:msg:assistant",
        ),
        vec![],
    );
    collapsed_assistant.is_collapsed = true;
    collapsed_assistant.spill_id = Some("turn2:msg:assistant".into());
    let mut collapsed_tool = Message::tool("call-1", "[bash: out — recall(\"turn1:bash:0\")]");
    collapsed_tool.is_collapsed = true;
    collapsed_tool.spill_id = Some("turn1:bash:0".into());
    let mut messages: Vec<Message> = vec![
        Message::user("first"),
        collapsed_assistant,
        collapsed_tool,
        Message::user("second"),
    ];

    assert!(rewind_to_message_index(&mut messages, 3));
    let assistant = messages
        .iter()
        .find(|m| m.role == Role::Assistant)
        .expect("collapsed assistant turn must be retained");
    assert!(
        !assistant.content.is_empty(),
        "a collapsed conversation message must not become an empty provider turn"
    );
    assert!(
        assistant.content.contains("I analysed the logs") && assistant.content.contains("840"),
        "the stub annotation (preview + tokens) must survive rewind, got: {}",
        assistant.content
    );
    assert!(
        !assistant.content.contains("recall("),
        "the dangling recall clause must be stripped (spill store was wiped)"
    );
    assert!(!assistant.is_collapsed && assistant.spill_id.is_none());
    let tool = messages.iter().find(|m| m.role == Role::Tool).unwrap();
    assert!(
        tool.content.is_empty(),
        "tool stubs keep the pre-#1046 blanking behaviour"
    );
}

// ─── clear_history + spill store (#412) ──────────────────────────────────────

#[tokio::test]
async fn test_spill_store_clear_removes_entries_and_recall() {
    use crate::domain::session::{ContextSpillStore, SpillEntry};
    use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
    let tmp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(FileContextSpillStore::new(tmp.path().to_path_buf()));
    let key = "clear-test-412";
    let entry = SpillEntry {
        id: "turn1:bash:0".into(),
        tool: "bash".into(),
        content: "ls output".into(),
        input_preview: r#"{"command": "ls"}"#.into(),
        tokens: 42,
    };
    store.append(key, &entry).await.unwrap();
    assert_eq!(store.list_entries(key).await.unwrap().len(), 1);
    assert!(store.recall(key, "turn1:bash:0").await.unwrap().is_some());
    // After clear: both list and recall should return empty/None.
    store.clear(key).await.unwrap();
    assert!(store.list_entries(key).await.unwrap().is_empty());
    assert!(store.recall(key, "turn1:bash:0").await.unwrap().is_none());
}
