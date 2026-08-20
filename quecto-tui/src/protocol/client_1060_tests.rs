//! #1060 / ADR-0008 part 2 — TUI wire contract for ref-based end-of-turn events.
//!
//! These tests compile against the current client types and FAIL until:
//! - AgentEnd retains non-empty messageRefs (not a unit variant that drops them)
//! - subagent_messages_appended retains non-empty messageRefs
//! - Command can issue get_message for on-demand lookup
//!
//! Loaded from `client.rs` via `#[path = "client_1060_tests.rs"]`.

use super::*;

fn refs_from_value(v: &serde_json::Value) -> Vec<String> {
    let candidates = [v.get("messageRefs"), v.get("message_refs")];
    for c in candidates.into_iter().flatten() {
        if let Some(arr) = c.as_array() {
            let refs: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    if let Some(s) = item.as_str() {
                        return Some(s.to_string());
                    }
                    item.get("id")
                        .and_then(|id| id.as_str())
                        .map(str::to_string)
                })
                .filter(|s| !s.is_empty())
                .collect();
            if !refs.is_empty() {
                return refs;
            }
        }
    }
    Vec::new()
}

/// #1060: turn_end message Value must preserve non-empty messageRefs for recovery.
#[test]
fn turn_end_preserves_non_empty_message_refs_in_message_value() {
    let json = r#"{
        "type":"turn_end",
        "message":{
            "role":"assistant",
            "content":"",
            "messageRefs":["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"],
            "contextTokens":40,
            "maxContextTokens":100
        },
        "toolResults":[]
    }"#;
    let event: Event = serde_json::from_str(json).expect("turn_end parses");
    match event {
        Event::TurnEnd { message } => {
            let refs = refs_from_value(&message);
            assert_eq!(
                refs,
                vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string()],
                "TurnEnd.message must preserve non-empty messageRefs (#1060)"
            );
            assert_eq!(message["contextTokens"], 40);
            assert_eq!(message["maxContextTokens"], 100);
        }
        _ => panic!("expected TurnEnd"),
    }
}

/// #1060 AC6: AgentEnd must retain non-empty messageRefs from the wire.
///
/// Pre-#1060 `AgentEnd` is a unit variant that drops `messages`/`messageRefs`.
/// This test fails until the variant retains structured non-empty refs.
/// Empty-ref arrays must NOT green (abandoned #1075 hole).
#[test]
fn agent_end_retains_non_empty_message_refs() {
    let expected = vec![
        "11111111-1111-1111-1111-111111111111".to_string(),
        "22222222-2222-2222-2222-222222222222".to_string(),
    ];
    let json = r#"{
        "type":"agent_end",
        "messages":[],
        "messageRefs":[
            "11111111-1111-1111-1111-111111111111",
            "22222222-2222-2222-2222-222222222222"
        ]
    }"#;
    let event: Event = serde_json::from_str(json).expect("agent_end parses");

    let refs = match &event {
        Event::AgentEnd { message_refs, .. } => message_refs.clone(),
        other => panic!("expected AgentEnd; got: {other:?}"),
    };
    assert_eq!(
        refs, expected,
        "AgentEnd must retain the exact non-empty messageRefs from the wire (#1060)"
    );
    assert!(
        !refs.is_empty() && refs.iter().all(|r| !r.is_empty()),
        "empty-ref arrays must not satisfy #1060"
    );
}

/// #1060 AC3: subagent_messages_appended must retain non-empty messageRefs.
#[test]
fn subagent_messages_appended_retains_non_empty_message_refs() {
    let expected = vec!["33333333-3333-3333-3333-333333333333".to_string()];
    let json = r#"{
        "type":"subagent_messages_appended",
        "agentId":"worker",
        "messages":[],
        "messageRefs":["33333333-3333-3333-3333-333333333333"]
    }"#;
    let event: Event = serde_json::from_str(json).unwrap_or(Event::Unknown);
    assert!(
        !matches!(event, Event::Unknown),
        "subagent_messages_appended must deserialize as a known event (#1060)"
    );
    let refs = match &event {
        Event::SubagentMessagesAppended { message_refs, .. } => message_refs.clone(),
        other => {
            panic!("subagent_messages_appended must retain structured message_refs; got: {other:?}")
        }
    };
    assert_eq!(refs, expected);
    assert!(refs.iter().all(|r| !r.is_empty()));
}

/// #1060 AC4: client must support a get_message command for on-demand lookup.
///
/// Inventories serializable Command type names. Fails on master (no GetMessage).
/// When Command::GetMessage lands, include it in the inventory and pin wire shape.
#[test]
fn get_message_command_is_available_for_on_demand_lookup() {
    // Keep this inventory in lockstep with Command variants used for recovery.
    // Adding GetMessage is the GREEN for this pin.
    let recoverable: Vec<Command> = vec![
        Command::GetState {
            id: None,
            agent_id: None,
        },
        Command::GetMessages {
            agent_id: None,
            id: None,
            before: None,
            count: None,
        },
        Command::GetMessagesTail {
            id: None,
            count: 1,
            agent_id: None,
        },
        Command::GetMessage {
            id: Some("gm-req-1".into()),
            message_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            agent_id: None,
            tool_call_id: None,
            offset: None,
            thinking_offset: None,
            limit: None,
        },
        Command::GetSessionStats { id: None },
    ];
    let names: Vec<&str> = recoverable.iter().map(|c| c.kind()).collect();
    assert!(
        names.contains(&"get_message"),
        "Command must support get_message for on-demand lookup (#1060 AC4); \
         known type names: {names:?}"
    );

    // When present, also pin the wire shape (stable messageId + request id).
    if let Some(cmd) = recoverable.into_iter().find(|c| c.kind() == "get_message") {
        let out = serde_json::to_string(&cmd).expect("serialize");
        assert!(
            out.contains("\"type\":\"get_message\""),
            "get_message type: {out}"
        );
    }
}

/// History get_messages payloads already carry ids once MessageView exposes them;
/// pin that the TUI Response path preserves them for recovery.
#[test]
fn get_messages_response_preserves_stable_message_ids() {
    let json = r#"{
        "type":"response",
        "command":"get_messages",
        "success":true,
        "data":{
            "messages":[
                {"id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","role":"user","content":"hi"},
                {"id":"bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee","role":"assistant","content":"yo"}
            ]
        }
    }"#;
    let event: Event = serde_json::from_str(json).unwrap();
    match event {
        Event::Response {
            data: Some(data), ..
        } => {
            let msgs = data["messages"].as_array().expect("messages array");
            for m in msgs {
                let id = m
                    .get("id")
                    .or_else(|| m.get("messageId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                assert!(
                    !id.is_empty(),
                    "history message must carry a non-empty stable id: {m}"
                );
            }
        }
        _ => panic!("expected Response"),
    }
}
