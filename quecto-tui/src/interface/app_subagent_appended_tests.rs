use crate::infrastructure::client::Event;
use crate::interface::ansi::strip_ansi;
use crate::interface::app::tui_harness::{
    TuiHarness, spawn_subagent_socket, subagent, subagent_with_socket, subagents_changed,
};

fn backfill_event(messages: Vec<serde_json::Value>) -> Event {
    Event::Response {
        id: Some("subagent-history".into()),
        command: "get_messages".into(),
        success: true,
        data: Some(serde_json::json!({ "messages": messages })),
        error: None,
    }
}

#[tokio::test]
async fn master_stream_appended_messages_populate_unfocused_child_session() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 1, 3)),
    )]));
    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "the master must remain the active view for the repro"
    );

    h.event(Event::SubagentMessagesAppended {
        agent_id: "worker".into(),
        messages: vec![
            serde_json::json!({
                "role": "assistant",
                "tool_calls": [{"id": "call-1", "function": {"name": "bash", "arguments": "{\"command\":\"echo hi\"}"}}],
                "content": null
            }),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": "call-1",
                "toolName": "bash",
                "content": "BACKGROUND_TOOL_OUTPUT"
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "BACKGROUND_CHILD_OUTPUT"
            }),
        ],
        message_refs: vec![
            "child-tool-call-ref".into(),
            "child-tool-result-ref".into(),
            "child-text-ref".into(),
        ],
    });

    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "routing a background child append must not focus/select the child"
    );
    assert_eq!(
        h.app_mut().session_chat_entry_count("worker"),
        Some(2),
        "the unfocused child's retained session should already contain the appended turn"
    );

    let frame = strip_ansi(
        &h.select(Some("worker"))
            .app_mut()
            .compose_frame()
            .join("\n"),
    );
    assert!(
        frame.contains("BACKGROUND_CHILD_OUTPUT") && frame.contains("BACKGROUND_TOOL_OUTPUT"),
        "selecting the child should reveal already-buffered content, not materialize it:\n{frame}"
    );
    assert_eq!(
        frame.matches("BACKGROUND_CHILD_OUTPUT").count(),
        1,
        "the already-buffered child output must not duplicate on select:\n{frame}"
    );
}

#[tokio::test]
async fn selected_child_direct_stream_is_not_duplicated_by_master_appended_event() {
    let socket = spawn_subagent_socket("worker-selected");
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent_with_socket(
        "worker-selected",
        "running",
        Some(("active", 1, 3)),
        Some(socket),
    )]));
    h.select(Some("worker-selected"));
    h.route(
        "worker-selected",
        Event::Token {
            token: "LIVE_DIRECT_STREAM".into(),
        },
    );
    h.route(
        "worker-selected",
        Event::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        },
    );
    assert_eq!(
        h.app_mut().session_chat_entry_count("worker-selected"),
        Some(1)
    );

    h.event(Event::SubagentMessagesAppended {
        agent_id: "worker-selected".into(),
        messages: vec![serde_json::json!({
            "role": "assistant",
            "id": "selected-same-turn",
            "content": "LIVE_DIRECT_STREAM"
        })],
        message_refs: vec!["selected-same-turn".into()],
    });

    assert_eq!(
        h.app_mut().session_chat_entry_count("worker-selected"),
        Some(1),
        "selected child direct stream/backfill remains authoritative; master append is ignored"
    );
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert_eq!(
        frame.matches("LIVE_DIRECT_STREAM").count(),
        1,
        "master-stream child append must not duplicate direct selected-child output:\n{frame}"
    );
}

#[tokio::test]
async fn synced_authoritative_child_ignores_parent_appended_crumbs() {
    let socket = spawn_subagent_socket("synced-child");
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent_with_socket(
        "synced-child",
        "running",
        Some(("active", 1, 3)),
        Some(socket),
    )]));
    h.app_mut()
        .note_sync_capability("synced-child", &serde_json::json!({"sync":1}));
    h.app_mut().route_sync_response(
        "synced-child",
        &serde_json::json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"ledger-turn","role":"assistant","content":"LEDGER_ONLY_OUTPUT"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );
    assert_eq!(
        h.app_mut().session_chat_entry_count("synced-child"),
        Some(1)
    );

    h.event(Event::SubagentMessagesAppended {
        agent_id: "synced-child".into(),
        messages: vec![serde_json::json!({
            "role": "assistant",
            "id": "parent-crumb",
            "content": "PARENT_CRUMB_OUTPUT"
        })],
        message_refs: vec!["parent-crumb".into()],
    });

    assert_eq!(
        h.app_mut().session_chat_entry_count("synced-child"),
        Some(1),
        "synced authoritative child transcript must not be mutated by parent crumbs"
    );
    let frame = strip_ansi(
        &h.select(Some("synced-child"))
            .app_mut()
            .compose_frame()
            .join(
                "
",
            ),
    );
    assert!(frame.contains("LEDGER_ONLY_OUTPUT"), "{frame}");
    assert!(!frame.contains("PARENT_CRUMB_OUTPUT"), "{frame}");
}

#[tokio::test]
async fn synced_authoritative_child_flushes_deferred_grandchild_notes_on_end() {
    let socket = spawn_subagent_socket("synced-grandchild-notes");
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent_with_socket(
        "synced-grandchild-notes",
        "running",
        Some(("active", 1, 3)),
        Some(socket),
    )]));
    h.app_mut()
        .note_sync_capability("synced-grandchild-notes", &serde_json::json!({"sync":1}));
    h.app_mut().route_sync_response(
        "synced-grandchild-notes",
        &serde_json::json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"ledger-turn","role":"assistant","content":"LEDGER_TRANSCRIPT"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    h.route("synced-grandchild-notes", Event::AgentStart);
    h.route(
        "synced-grandchild-notes",
        Event::SubagentNotification {
            agent_id: "grandchild".into(),
            sequence: 1,
            message: "grandchild finished".into(),
        },
    );
    h.route(
        "synced-grandchild-notes",
        Event::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        },
    );

    let frame = strip_ansi(
        &h.select(Some("synced-grandchild-notes"))
            .app_mut()
            .compose_frame()
            .join("\n"),
    );
    assert!(frame.contains("LEDGER_TRANSCRIPT"), "{frame}");
    assert!(
        frame.contains("grandchild finished"),
        "synced authoritative end-of-turn must flush deferred grandchild notes:\n{frame}"
    );
}

#[tokio::test]
async fn appended_messages_update_inactive_sibling_without_stealing_focus() {
    let active_socket = spawn_subagent_socket("active-child");
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent_with_socket(
            "active-child",
            "running",
            Some(("active", 1, 3)),
            Some(active_socket),
        ),
        subagent("background-child", "running", Some(("active", 1, 3))),
    ]));
    h.select(Some("active-child"));

    h.event(Event::SubagentMessagesAppended {
        agent_id: "background-child".into(),
        messages: vec![serde_json::json!({
            "role": "assistant",
            "id": "background-turn",
            "content": "SIBLING_BACKGROUND_OUTPUT"
        })],
        message_refs: vec!["background-turn".into()],
    });

    assert_eq!(
        h.app_mut().active_agent_id(),
        Some("active-child"),
        "a sibling's background transcript update must not steal focus"
    );
    assert_eq!(
        h.app_mut().session_chat_entry_count("background-child"),
        Some(1),
        "inactive sibling session should be warmed by the master-stream append"
    );
    let frame = strip_ansi(
        &h.select(Some("background-child"))
            .app_mut()
            .compose_frame()
            .join("\n"),
    );
    assert_eq!(
        frame.matches("SIBLING_BACKGROUND_OUTPUT").count(),
        1,
        "{frame}"
    );
}

#[tokio::test]
async fn duplicate_appended_message_refs_are_ignored_for_unfocused_child() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("worker", "running", None)]));
    for _ in 0..2 {
        h.event(Event::SubagentMessagesAppended {
            agent_id: "worker".into(),
            messages: vec![serde_json::json!({
                "role": "assistant",
                "content": "DEDUPED_CHILD_OUTPUT"
            })],
            message_refs: vec!["dup-turn".into()],
        });
    }

    assert_eq!(h.app_mut().session_chat_entry_count("worker"), Some(1));
    let frame = strip_ansi(
        &h.select(Some("worker"))
            .app_mut()
            .compose_frame()
            .join("\n"),
    );
    assert_eq!(frame.matches("DEDUPED_CHILD_OUTPUT").count(), 1, "{frame}");
}

#[tokio::test]
async fn refs_only_appended_event_requests_child_message_recovery() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("worker", "running", None)]));

    h.event(Event::SubagentMessagesAppended {
        agent_id: "worker".into(),
        messages: vec![],
        message_refs: vec!["ref-only-child-turn".into()],
    });

    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|line| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            v.get("type").and_then(|t| t.as_str()) == Some("get_message")
                && v.get("agent_id").and_then(|id| id.as_str()) == Some("worker")
                && v.get("messageId").and_then(|id| id.as_str()) == Some("ref-only-child-turn")
        }),
        "refs-only child append should recover through the master get_message path: {cmds:?}"
    );
}

#[tokio::test]
async fn later_direct_backfill_replaces_master_appended_prefix_without_duplicates() {
    let socket = spawn_subagent_socket("worker-backfill");
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent_with_socket(
        "worker-backfill",
        "idle",
        None,
        Some(socket),
    )]));
    h.event(Event::SubagentMessagesAppended {
        agent_id: "worker-backfill".into(),
        messages: vec![serde_json::json!({
            "role": "assistant",
            "id": "already-warmed",
            "content": "WARMED_CHILD_TURN"
        })],
        message_refs: vec!["already-warmed".into()],
    });

    h.select(Some("worker-backfill"));
    h.route(
        "worker-backfill",
        backfill_event(vec![serde_json::json!({
            "role": "assistant",
            "id": "already-warmed",
            "content": "WARMED_CHILD_TURN"
        })]),
    );

    assert_eq!(
        h.app_mut().session_chat_entry_count("worker-backfill"),
        Some(1),
        "direct selected-child backfill must replace the warmed master-stream prefix"
    );
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert_eq!(frame.matches("WARMED_CHILD_TURN").count(), 1, "{frame}");
}

#[tokio::test]
async fn direct_backfill_after_later_warm_append_preserves_old_history() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("worker", "idle", None)]));
    h.select(Some("worker"));
    h.route(
        "worker",
        backfill_event(vec![serde_json::json!({
            "role": "assistant", "id": "old", "content": "OLD_TURN"
        })]),
    );
    h.select(None);
    h.event(Event::SubagentMessagesAppended {
        agent_id: "worker".into(),
        messages: vec![serde_json::json!({
            "role": "assistant", "id": "new", "content": "NEW_TURN"
        })],
        message_refs: vec!["new".into()],
    });
    h.select(Some("worker"));
    h.route(
        "worker",
        backfill_event(vec![
            serde_json::json!({"role": "assistant", "id": "old", "content": "OLD_TURN"}),
            serde_json::json!({"role": "assistant", "id": "new", "content": "NEW_TURN"}),
        ]),
    );

    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert_eq!(frame.matches("OLD_TURN").count(), 1, "{frame}");
    assert_eq!(frame.matches("NEW_TURN").count(), 1, "{frame}");
}

#[tokio::test]
async fn delayed_master_append_after_direct_backfill_is_deduplicated() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("worker", "idle", None)]));
    h.select(Some("worker"));
    h.route(
        "worker",
        backfill_event(vec![serde_json::json!({
            "role": "assistant", "id": "m1", "content": "ONCE"
        })]),
    );
    h.select(None);
    h.event(Event::SubagentMessagesAppended {
        agent_id: "worker".into(),
        messages: vec![serde_json::json!({
            "role": "assistant", "id": "m1", "content": "ONCE"
        })],
        message_refs: vec!["m1".into()],
    });

    h.select(Some("worker"));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert_eq!(frame.matches("ONCE").count(), 1, "{frame}");
}
