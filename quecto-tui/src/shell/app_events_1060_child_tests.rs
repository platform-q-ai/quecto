//! #1060 review — child-path recovery + coverage-gap tests, split out of
//! app_events_1060_tests.rs to respect the 750-line file cap.
//!
//! Loaded from `app.rs` via `#[path = "app_events_1060_child_tests.rs"]`.

use crate::components::ansi::strip_ansi;
use crate::components::chat::ChatEntry;
use crate::components::component::Component;
use crate::protocol::client::Event;
use crate::shell::app::App;
use crate::shell::app::app_events::recovered_chat_entries;
use crate::shell::app::tui_harness::{TuiHarness, subagent, subagents_changed};

fn chat_text(app: &mut App) -> String {
    app.master_session
        .chat
        .render(120)
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_get_message_cmd(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| {
            v.get("type")
                .and_then(|t| t.as_str())
                .map(|s| s == "get_message")
        })
        .unwrap_or(false)
}

fn get_message_ids(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| is_get_message_cmd(l))
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            v.get("id").and_then(|i| i.as_str()).map(str::to_string)
        })
        .collect()
}

/// #1060 review F1: a child mid-turn miss recovers by routing get_message via
/// the MASTER (agent_id = ending child), even when that child is not selected,
/// and the master-stream response reconciles into that child's chat.
#[tokio::test]
async fn child_mid_turn_miss_recovers_via_master_and_reconciles() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 1, 3)),
    )]));
    // Master stays selected — the child "worker" is a background session.
    let ref_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    h.route("worker", Event::AgentStart);
    h.route(
        "worker",
        Event::Token {
            token: "…".into()
        },
    );
    h.route(
        "worker",
        Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant", "content": "",
                "messageRefs": [ref_id], "contentLength": 64
            }),
        },
    );
    let cmds = h.drain_commands().await;
    let recovery: Vec<&String> = cmds.iter().filter(|l| is_get_message_cmd(l)).collect();
    assert!(
        !recovery.is_empty(),
        "child mid-turn miss must issue a recovery get_message: {cmds:?}"
    );
    assert!(
        recovery.iter().all(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["agent_id"].as_str() == Some("worker") && v["messageId"].as_str() == Some(ref_id)
        }),
        "recovery must target the ending child by agent_id, routed via master: {recovery:?}"
    );
    let req_id = get_message_ids(&cmds)[0].clone();
    // Forwarded child message returns on the master stream → child's chat.
    h.app_mut().handle_event(Event::Response {
        id: Some(req_id),
        command: "get_message".into(),
        success: true,
        data: Some(serde_json::json!({
            "id": ref_id,
            "role": "assistant",
            "content": "FULL_CHILD_BODYxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        })),
        error: None,
    });
    let frame = strip_ansi(
        &h.select(Some("worker"))
            .app_mut()
            .compose_frame()
            .join("\n"),
    );
    assert!(
        frame.contains("FULL_CHILD_BODY"),
        "recovered child content must reconcile into the child's chat:\n{frame}"
    );
}

/// #1060 review F2: a child text turn after earlier tool turns must NOT refetch
/// — the per-turn tool count (reset each turn), not lifetime, drives cardinality.
#[tokio::test]
async fn child_streamed_turn_after_prior_tools_does_not_refetch() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 1, 3)),
    )]));
    // Turn 1: a tool-using turn leaves tool entries in the child's lifetime chat.
    h.route("worker", Event::AgentStart);
    h.route(
        "worker",
        Event::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({}),
        },
    );
    h.route(
        "worker",
        Event::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
            is_error: false,
        },
    );
    h.route(
        "worker",
        Event::Token {
            token: "first".into(),
        },
    );
    h.route(
        "worker",
        Event::TurnEnd {
            message: serde_json::json!({"role":"assistant","content":"","messageRefs":[
                "11111111-1111-1111-1111-111111111111",
                "22222222-2222-2222-2222-222222222222",
                "33333333-3333-3333-3333-333333333333"
            ]}),
        },
    );
    let _ = h.drain_commands().await;
    // Turn 2: a plain streamed text turn — zero tools THIS turn, fully observed.
    h.route("worker", Event::AgentStart);
    h.route(
        "worker",
        Event::Token {
            token: "second answer".into(),
        },
    );
    h.route(
        "worker",
        Event::TurnEnd {
            message: serde_json::json!({
                "role":"assistant","content":"",
                "messageRefs":["44444444-4444-4444-4444-444444444444"]
            }),
        },
    );
    let cmds = h.drain_commands().await;
    assert!(
        !cmds.iter().any(|l| is_get_message_cmd(l)),
        "a child text turn after prior tool turns must not refetch (per-turn \
         count, not lifetime); got: {cmds:?}"
    );
}

/// Child recovery must judge only the active turn range; a previous assistant
/// in the same child session must not suppress recovery for a lost/empty turn.
#[tokio::test]
async fn child_empty_turn_recovery_ignores_previous_assistant() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 1, 3)),
    )]));

    h.route("worker", Event::AgentStart);
    h.route(
        "worker",
        Event::Token {
            token: "prior complete".into(),
        },
    );
    h.route(
        "worker",
        Event::TurnEnd {
            message: serde_json::json!({
                "role":"assistant", "content":"prior complete", "messageRefs":["prior-ref"]
            }),
        },
    );
    let _ = h.drain_commands().await;

    let ref_id = "lost-child-turn-ref";
    h.route("worker", Event::AgentStart);
    h.route(
        "worker",
        Event::TurnEnd {
            message: serde_json::json!({
                "role":"assistant", "content":"", "messageRefs":[ref_id], "contentLength": 9
            }),
        },
    );

    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().filter(|l| is_get_message_cmd(l)).any(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["agent_id"].as_str() == Some("worker") && v["messageId"].as_str() == Some(ref_id)
        }),
        "empty child turn must recover despite previous assistant: {cmds:?}"
    );
}

/// Child recovery de-dupe is child-scoped: two children may legitimately share
/// the same opaque message ref, so a pending recovery for one child must not
/// suppress the other's request.
#[tokio::test]
async fn child_recovery_dedupe_allows_same_ref_for_different_children() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("child-a", "running", Some(("active", 1, 3))),
        subagent("child-b", "running", Some(("active", 1, 3))),
    ]));

    let ref_id = "shared-child-ref";
    h.route("child-a", Event::AgentStart);
    h.route(
        "child-a",
        Event::TurnEnd {
            message: serde_json::json!({
                "role":"assistant", "content":"", "messageRefs":[ref_id], "contentLength": 20
            }),
        },
    );
    let first_cmds = h.drain_commands().await;
    assert!(
        first_cmds
            .iter()
            .filter(|l| is_get_message_cmd(l))
            .any(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["agent_id"].as_str() == Some("child-a") && v["messageId"].as_str() == Some(ref_id)
            }),
        "first child must start recovery: {first_cmds:?}"
    );

    h.route("child-b", Event::AgentStart);
    h.route(
        "child-b",
        Event::TurnEnd {
            message: serde_json::json!({
                "role":"assistant", "content":"", "messageRefs":[ref_id], "contentLength": 20
            }),
        },
    );
    let second_cmds = h.drain_commands().await;
    assert!(
        second_cmds
            .iter()
            .filter(|l| is_get_message_cmd(l))
            .any(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["agent_id"].as_str() == Some("child-b") && v["messageId"].as_str() == Some(ref_id)
            }),
        "second child with colliding ref must issue its own scoped recovery: {second_cmds:?}"
    );
}

/// #1060: a tool message recovered with `isError` must reconstruct as an errored
/// tool box, not a clean one.
#[test]
fn recovered_chat_entries_propagates_is_error() {
    use std::collections::HashMap;
    let refs = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let mut responses: HashMap<String, serde_json::Value> = HashMap::new();
    responses.insert(
        "a".into(),
        serde_json::json!({
            "role":"assistant","content":"",
            "toolCalls":[{"id":"t1","name":"bash","arguments":"{}"}]
        }),
    );
    responses.insert(
        "b".into(),
        serde_json::json!({"role":"tool","toolCallId":"t1","content":"boom","is_error":true}),
    );
    responses.insert(
        "c".into(),
        serde_json::json!({"role":"tool","toolCallId":"t2","content":"boom","isError":true}),
    );
    let entries = recovered_chat_entries(&refs, &responses);
    assert!(
        entries.iter().any(|e| matches!(
            e,
            ChatEntry::ToolExecution { is_error: true, result: Some(r), .. } if r == "boom"
        )),
        "recovered tool entry must carry isError/is_error = true"
    );
    assert!(
        entries.iter().any(|e| matches!(
            e,
            ChatEntry::ToolExecution {
                tool_call_id,
                args,
                result: Some(r),
                is_error: true,
                ..
            } if tool_call_id == "t2" && args.is_empty() && r == "boom"
        )),
        "orphan recovered tool entry must preserve empty fallback args"
    );
}

/// #1103 review: a tool result with no call id is incomplete recovery metadata;
/// it must not create a phantom tool box in the chat transcript.
#[test]
fn recovered_chat_entries_ignores_empty_tool_call_id() {
    use std::collections::HashMap;
    let refs = vec!["a".to_string()];
    let mut responses: HashMap<String, serde_json::Value> = HashMap::new();
    responses.insert(
        "a".into(),
        serde_json::json!({"role":"tool","toolCallId":"","toolName":"bash","content":"orphan"}),
    );

    let entries = recovered_chat_entries(&refs, &responses);

    assert!(
        entries.is_empty(),
        "empty toolCallId must not create a tool box"
    );
}

/// #1060 review: a failed recovery response must abandon the whole batch — the
/// turn stays as-streamed and a late sibling response can no longer apply.
#[tokio::test]
async fn failed_recovery_response_abandons_batch() {
    let r1 = "11111111-1111-1111-1111-111111111111";
    let r2 = "22222222-2222-2222-2222-222222222222";
    let mut h = TuiHarness::new().await;
    {
        let a = h.app_mut();
        a.handle_event(Event::AgentStart);
        a.handle_event(Event::Token {
            token: "partial".into(),
        });
        // contentLength >> held text forces recovery of both refs.
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role":"assistant","content":"",
                "messageRefs":[r1, r2], "contentLength": 200
            }),
        });
    }
    let cmds = h.drain_commands().await;
    let ids = get_message_ids(&cmds);
    assert!(
        ids.len() >= 2,
        "both missing refs must be fetched to set up the batch: {cmds:?}"
    );
    // Fail the first ref → batch abandoned, sibling pending dropped.
    h.app_mut().handle_event(Event::Response {
        id: Some(ids[0].clone()),
        command: "get_message".into(),
        success: false,
        data: None,
        error: Some("message not found".into()),
    });
    // A late success for the sibling must NOT apply (batch already abandoned).
    h.app_mut().handle_event(Event::Response {
        id: Some(ids[1].clone()),
        command: "get_message".into(),
        success: true,
        data: Some(serde_json::json!({"id": r2, "role":"assistant","content":"LATE_APPLY"})),
        error: None,
    });
    let text = chat_text(h.app_mut());
    assert!(
        text.contains("partial"),
        "turn must stay as-streamed after a failed recovery:\n{text}"
    );
    assert!(
        !text.contains("LATE_APPLY"),
        "an abandoned batch must not later apply a sibling response:\n{text}"
    );
}
