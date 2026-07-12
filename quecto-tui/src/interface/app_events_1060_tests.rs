//! #1060 / ADR-0008 part 2 — TUI recovery behaviour for ref-based end-of-turn.
//!
//! Recovery design (mandatory — do NOT re-land #1075 fetch-all + append-all):
//!   - Common streamed case: non-empty refs, ZERO fetches, ZERO duplicates.
//!   - Miss/partial: fetch only missing content; RECONCILE/REPLACE at turn position.
//!   - Request-id gating on recovery responses.
//!   - Reconstruct ALL roles (assistant text, tool-call, tool results).
//!
//! Uses TuiHarness so drain_commands() can observe fetch behaviour.
//! Loaded from `app.rs` via `#[path = "app_events_1060_tests.rs"]`.

use super::tui_harness::TuiHarness;
use super::*;

fn chat_text(app: &mut App) -> String {
    let lines = app.master_session.chat.render(120);
    lines
        .iter()
        .map(|l| super::app_methods::strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_get_message_cmd(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    v.get("type").and_then(|t| t.as_str()) == Some("get_message")
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

/// #1060 AC2: fully streamed text turn → render full text, zero get_message fetches.
#[tokio::test]
async fn streamed_text_turn_renders_without_message_fetches_at_end_of_turn() {
    let mut h = TuiHarness::new().await;
    {
        let a = h.app_mut();
        a.handle_event(Event::AgentStart);
        a.handle_event(Event::Token {
            token: "Hello".into(),
        });
        a.handle_event(Event::Token {
            token: " world".into(),
        });
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "messageRefs": ["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"],
                "contextTokens": 40,
                "maxContextTokens": 100
            }),
        });
        a.handle_event(Event::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        });
        let frame = chat_text(a);
        assert!(
            frame.contains("Hello world"),
            "streamed turn must show full text:\n{frame}"
        );
        assert_eq!(
            frame.matches("Hello world").count(),
            1,
            "must not double-render the assistant bubble:\n{frame}"
        );
    }
    let cmds = h.drain_commands().await;
    assert!(
        !cmds.iter().any(|l| is_get_message_cmd(l)),
        "common streamed path must issue ZERO get_message fetches (#1060); got: {cmds:?}"
    );
}

/// #1060 AC7: footer still updates from ref-based turn_end metadata.
#[tokio::test]
async fn ref_based_turn_end_updates_footer_context_without_fetches() {
    let mut h = TuiHarness::new().await;
    {
        let a = h.app_mut();
        a.handle_event(Event::Token { token: "ok".into() });
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "messageRefs": ["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"],
                "contextTokens": 40_000,
                "maxContextTokens": 200_000
            }),
        });
        let rendered = a.master_session.footer.render(80).join("\n");
        // Format matches existing footer tests (e.g. 12k/200k for 12_000/200_000).
        assert!(
            rendered.contains("40k/200k"),
            "footer must use turn_end context metadata: {rendered}"
        );
    }
    let cmds = h.drain_commands().await;
    assert!(
        !cmds.iter().any(|l| is_get_message_cmd(l)),
        "footer path must not fetch messages: {cmds:?}"
    );
}

/// #1060 AC2: mid-turn miss → fetch only missing refs, reconcile (no duplicates).
#[tokio::test]
async fn mid_turn_connect_fetches_missing_refs_and_reconciles_without_duplicates() {
    let mut h = TuiHarness::new().await;
    let ref_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    {
        let a = h.app_mut();
        // Mid-turn: agent running, but we missed the early tokens of the active turn.
        a.handle_event(Event::AgentStart);
        // Partial live token only — missing the bulk of the assistant message.
        a.handle_event(Event::Token {
            token: "…".into()
        });
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "messageRefs": [ref_id],
                "contextTokens": 10,
                "maxContextTokens": 100
            }),
        });
    }
    // End-of-turn with non-empty refs for content we do not fully hold must
    // trigger fetch-on-miss (not zero-fetch common path).
    let cmds = h.drain_commands().await;
    let fetches: Vec<&String> = cmds.iter().filter(|l| is_get_message_cmd(l)).collect();
    assert!(
        !fetches.is_empty(),
        "mid-turn miss must issue get_message for missing refs (#1060); cmds={cmds:?}"
    );
    assert!(
        fetches.iter().all(|l| l.contains(ref_id)),
        "fetch must target the missing ref {ref_id}: {fetches:?}"
    );
    let req_ids = get_message_ids(&cmds);
    assert!(
        !req_ids.is_empty() && req_ids.iter().all(|id| !id.is_empty()),
        "recovery get_message must carry a request id for gating: {cmds:?}"
    );
    let recovery_id = req_ids[0].clone();

    // Deliver recovery response gated by request id — full content for the ref.
    {
        let a = h.app_mut();
        a.handle_event(Event::Response {
            id: Some(recovery_id.clone()),
            command: "get_message".into(),
            success: true,
            data: Some(serde_json::json!({
                "id": ref_id,
                "role": "assistant",
                "content": "FULL_RECOVERED_ASSISTANT_BODY"
            })),
            error: None,
        });
        let frame = chat_text(a);
        assert!(
            frame.contains("FULL_RECOVERED_ASSISTANT_BODY"),
            "recovery must reconcile full assistant content into the active turn:\n{frame}"
        );
        assert_eq!(
            frame.matches("FULL_RECOVERED_ASSISTANT_BODY").count(),
            1,
            "reconcile must not duplicate bubbles (#1075 regression):\n{frame}"
        );

        // Wrong request id must be ignored.
        a.handle_event(Event::Response {
            id: Some("not-our-recovery-id".into()),
            command: "get_message".into(),
            success: true,
            data: Some(serde_json::json!({
                "id": ref_id,
                "role": "assistant",
                "content": "SHOULD_NOT_APPEAR"
            })),
            error: None,
        });
        let frame2 = chat_text(a);
        assert!(
            !frame2.contains("SHOULD_NOT_APPEAR"),
            "recovery responses must be request-id gated:\n{frame2}"
        );
    }
}

/// #1060 AC2: mid-turn recovery reconstructs tool-call + tool-result roles.
#[tokio::test]
async fn mid_turn_recovery_reconstructs_tool_roles_in_order() {
    let mut h = TuiHarness::new().await;
    let call_ref = "bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee";
    let tool_ref = "cccccccc-cccc-dddd-eeee-ffffffffffff";
    let text_ref = "dddddddd-dddd-eeee-ffff-000000000000";
    {
        let a = h.app_mut();
        a.handle_event(Event::AgentStart);
        // Missed tool_execution_* events entirely.
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "messageRefs": [call_ref, tool_ref, text_ref]
            }),
        });
    }
    let cmds = h.drain_commands().await;
    let fetches: Vec<&String> = cmds.iter().filter(|l| is_get_message_cmd(l)).collect();
    assert!(
        fetches.len() >= 3,
        "must fetch each missing role ref (tool-call, tool-result, text); got {fetches:?}"
    );

    // Match recovery responses by outbound messageId, not fetch index.
    let mut by_message_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for line in fetches {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let mid = v
            .get("messageId")
            .or_else(|| v.get("message_id"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let rid = v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        assert!(
            !mid.is_empty() && !rid.is_empty(),
            "get_message must carry id+messageId: {line}"
        );
        by_message_id.insert(mid, rid);
    }
    for expected in [call_ref, tool_ref, text_ref] {
        assert!(
            by_message_id.contains_key(expected),
            "missing get_message for ref {expected}; map={by_message_id:?}"
        );
    }

    let payloads = [
        (
            call_ref,
            serde_json::json!({
                "id": call_ref,
                "role": "assistant",
                "content": "",
                "toolCalls": [{"id":"c1","name":"bash","arguments":"{}"}]
            }),
        ),
        (
            tool_ref,
            serde_json::json!({
                "id": tool_ref,
                "role": "tool",
                "content": "tool-result-body",
                "toolCallId": "c1",
                "toolName": "bash"
            }),
        ),
        (
            text_ref,
            serde_json::json!({
                "id": text_ref,
                "role": "assistant",
                "content": "final-text-after-tool"
            }),
        ),
    ];
    {
        let a = h.app_mut();
        for (mid, data) in payloads {
            let rid = by_message_id.get(mid).unwrap().clone();
            a.handle_event(Event::Response {
                id: Some(rid),
                command: "get_message".into(),
                success: true,
                data: Some(data),
                error: None,
            });
        }
        let frame = chat_text(a);
        // Bash tools render as `$ …` not the bare name "bash"; pin the result body.
        assert!(
            frame.contains("tool-result-body"),
            "must reconstruct tool-result body:\n{frame}"
        );
        assert!(
            frame.contains("final-text-after-tool"),
            "must reconstruct final assistant text:\n{frame}"
        );
        assert_eq!(
            frame.matches("final-text-after-tool").count(),
            1,
            "no duplicate final text:\n{frame}"
        );
    }
}

/// #1060 AC2: fully streamed tool turn → zero get_message at end-of-turn.
#[tokio::test]
async fn streamed_tool_turn_renders_without_message_fetches_at_end_of_turn() {
    let mut h = TuiHarness::new().await;
    {
        let a = h.app_mut();
        a.handle_event(Event::AgentStart);
        a.handle_event(Event::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            args: serde_json::json!({}),
        });
        a.handle_event(Event::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
            is_error: false,
        });
        a.handle_event(Event::Token {
            token: "done".into(),
        });
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "messageRefs": [
                    "bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "cccccccc-cccc-dddd-eeee-ffffffffffff",
                    "dddddddd-dddd-eeee-ffff-000000000000"
                ]
            }),
        });
        let frame = chat_text(a);
        assert!(
            frame.contains("bash") || frame.contains("ok"),
            "streamed tool turn must show tool activity:\n{frame}"
        );
        assert!(
            frame.contains("done"),
            "streamed tool turn must show final text:\n{frame}"
        );
    }
    let cmds = h.drain_commands().await;
    assert!(
        !cmds.iter().any(|l| is_get_message_cmd(l)),
        "streamed tool common path must issue ZERO get_message fetches; got: {cmds:?}"
    );
}
