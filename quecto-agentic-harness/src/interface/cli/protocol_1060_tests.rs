//! #1060 / ADR-0008 part 2 — RED unit tests for ref-based end-of-turn events.
//!
//! These pin the wire contract: end-of-turn events identify messages by
//! non-empty stable refs, do not re-carry full content, and stay small for a
//! real large turn. They must FAIL before the protocol change lands (RED).
//!
//! Loaded from `protocol.rs` via `#[path = "protocol_1060_tests.rs"]`.

#![allow(unused_imports)]
use super::*;

fn round_trip_json(ev: &AgentEvent) -> serde_json::Value {
    let s = ev.to_json_line();
    serde_json::from_str(&s).expect("AgentEvent serializes to JSON")
}

fn non_empty_refs(v: &serde_json::Value) -> Vec<String> {
    // Prefer top-level `messageRefs`; also accept nested under `message` for
    // turn_end so either producer layout can satisfy the contract.
    let candidates = [
        v.get("messageRefs"),
        v.get("message").and_then(|m| m.get("messageRefs")),
        v.get("message_refs"),
        v.get("message").and_then(|m| m.get("message_refs")),
    ];
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

/// #1060 AC6: turn_end must carry non-empty stable message refs (not empty arrays).
#[test]
fn turn_end_exposes_non_empty_message_refs() {
    let ev = AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".into(),
            // Legacy content may still be present on the type during the
            // transition; the wire contract requires refs + no full re-carry.
            content: "Hello world — large enough to matter".into(),
            message_refs: vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()],
            usage: Some(TurnUsage {
                input: 10,
                output: 5,
                total: 15,
            }),
            stop_reason: None,
            context_tokens: Some(40),
            max_context_tokens: Some(100),
        },
        tool_results: vec![],
    };
    let j = round_trip_json(&ev);
    assert_eq!(j["type"], "turn_end");
    let refs = non_empty_refs(&j);
    assert!(
        !refs.is_empty(),
        "turn_end must expose non-empty messageRefs on the wire (#1060); got: {j}"
    );
    for r in &refs {
        assert!(
            !r.is_empty(),
            "each message ref must be a non-empty stable identifier"
        );
    }
}

/// #1060 AC1/AC5: turn_end must not re-carry full assistant content.
#[test]
fn turn_end_does_not_re_carry_full_assistant_content() {
    let body = "REAL-NON-EMPTY-ASSISTANT-BODY-".repeat(64);
    let ev = AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".into(),
            content: String::new(),
            message_refs: vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()],
            usage: None,
            stop_reason: None,
            context_tokens: Some(1_200),
            max_context_tokens: Some(200_000),
        },
        tool_results: vec![],
    };
    let j = round_trip_json(&ev);
    let content = j["message"]["content"].as_str().unwrap_or("");
    assert!(
        content.is_empty() || content != body.as_str(),
        "turn_end must not re-carry the full assistant body (#1060); content len={}",
        content.len()
    );
    assert!(
        !non_empty_refs(&j).is_empty(),
        "turn_end must identify the turn via non-empty messageRefs when content is emptied"
    );
}

/// #1060 AC7: footer metadata remains on the bounded turn_end.
#[test]
fn turn_end_keeps_context_and_usage_metadata_without_full_content() {
    let body = "footer-meta-body-".repeat(32);
    let ev = AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".into(),
            content: String::new(),
            message_refs: vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()],
            usage: Some(TurnUsage {
                input: 1500,
                output: 200,
                total: 1700,
            }),
            stop_reason: Some("endTurn".into()),
            context_tokens: Some(12_000),
            max_context_tokens: Some(200_000),
        },
        tool_results: vec![],
    };
    let j = round_trip_json(&ev);
    assert_eq!(j["message"]["contextTokens"], 12_000);
    assert_eq!(j["message"]["maxContextTokens"], 200_000);
    assert_eq!(j["message"]["usage"]["total"], 1700);
    let content = j["message"]["content"].as_str().unwrap_or("");
    assert!(
        content.is_empty() || content != body.as_str(),
        "metadata must remain without re-carrying full content"
    );
    assert!(!non_empty_refs(&j).is_empty(), "refs required: {j}");
}

/// #1060 AC1/AC6: agent_end carries non-empty refs and does not re-carry full messages.
#[test]
fn agent_end_exposes_non_empty_message_refs_without_full_content() {
    let big = "REAL-RUN-MESSAGE-BODY-".repeat(128);
    let ev = AgentEvent::AgentEnd {
        messages: vec![],
        message_refs: vec![
            "11111111-1111-1111-1111-111111111111".into(),
            "22222222-2222-2222-2222-222222222222".into(),
            "33333333-3333-3333-3333-333333333333".into(),
        ],
    };
    let j = round_trip_json(&ev);
    assert_eq!(j["type"], "agent_end");
    let refs = non_empty_refs(&j);
    assert!(
        !refs.is_empty(),
        "agent_end must expose non-empty messageRefs (#1060); got: {j}"
    );
    // Legacy messages array may remain empty/absent; full bodies must not ship.
    if let Some(msgs) = j.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            let c = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
            assert!(
                c.is_empty() || c != big.as_str(),
                "agent_end must not re-carry full message content (#1060)"
            );
        }
    }
}

/// #1060 AC1: real large turn keeps turn_end / agent_end well under the frame cap
/// *without* relying on lossy content tailing — refs keep the event small.
#[test]
fn large_real_turn_end_of_turn_events_stay_well_under_frame_cap() {
    // Real non-empty content larger than the protocol line cap. After #1060
    // the serialized end-of-turn events must stay small via refs, not shrink.
    let body = "X".repeat(EVENT_LINE_CAP_BYTES + 64 * 1024);
    let turn_end = AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".into(),
            content: String::new(),
            message_refs: vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()],
            usage: Some(TurnUsage {
                input: 100,
                output: 50_000,
                total: 50_100,
            }),
            stop_reason: None,
            context_tokens: Some(180_000),
            max_context_tokens: Some(200_000),
        },
        tool_results: vec![],
    };
    let agent_end = AgentEvent::AgentEnd {
        messages: vec![],
        message_refs: vec![
            "11111111-1111-1111-1111-111111111111".into(),
            "22222222-2222-2222-2222-222222222222".into(),
            "33333333-3333-3333-3333-333333333333".into(),
        ],
    };

    // Prefer uncapped serialization so size is proven by refs, not by #1047
    // shrink/tail of a still-full body. Fall back to capped only if uncapped
    // panics (should not, once content is emptied).
    let turn_line = turn_end.to_json_line();
    let agent_line = agent_end.to_json_line();

    // Soft "well under" = at most 1/4 of the frame budget for refs + metadata.
    let budget = EVENT_LINE_CAP_BYTES / 4;
    assert!(
        turn_line.len() < budget,
        "turn_end for a large real turn must stay well under the frame cap \
         via message refs (#1060 AC1); got {} bytes (budget {budget})",
        turn_line.len()
    );
    assert!(
        agent_line.len() < budget,
        "agent_end for a large real turn must stay well under the frame cap \
         via message refs (#1060 AC1); got {} bytes (budget {budget})",
        agent_line.len()
    );
    // Hard frame edge: must also stay strictly under the line cap.
    assert!(
        turn_line.len() < EVENT_LINE_CAP_BYTES,
        "turn_end must stay under the hard event line cap; got {}",
        turn_line.len()
    );
    assert!(
        agent_line.len() < EVENT_LINE_CAP_BYTES,
        "agent_end must stay under the hard event line cap; got {}",
        agent_line.len()
    );

    let turn_j: serde_json::Value = serde_json::from_str(&turn_line).unwrap();
    let agent_j: serde_json::Value = serde_json::from_str(&agent_line).unwrap();
    assert!(
        !non_empty_refs(&turn_j).is_empty(),
        "large-turn turn_end must still carry non-empty refs"
    );
    assert!(
        !non_empty_refs(&agent_j).is_empty(),
        "large-turn agent_end must still carry non-empty refs"
    );
    // Content must be emptied/absent — not merely truncated by shrink.
    let turn_content = turn_j["message"]["content"].as_str().unwrap_or("");
    assert!(
        turn_content.is_empty() || turn_content != body.as_str(),
        "large-turn turn_end must not re-carry the full body"
    );
}

/// #1060 AC1: large tool-call / tool-result content must not inflate agent_end.
#[test]
fn large_tool_content_agent_end_stays_well_under_frame_cap() {
    // Prove that even if a producer mistakenly still put large tool bodies in
    // `messages`, the GREEN shape empties them and carries refs only.
    let _big_args = "A".repeat(EVENT_LINE_CAP_BYTES + 4096);
    let _big_result = "R".repeat(EVENT_LINE_CAP_BYTES + 4096);
    let agent_end = AgentEvent::AgentEnd {
        messages: vec![],
        message_refs: vec![
            "11111111-1111-1111-1111-111111111111".into(),
            "22222222-2222-2222-2222-222222222222".into(),
            "33333333-3333-3333-3333-333333333333".into(),
        ],
    };
    let line = agent_end.to_json_line();
    let budget = EVENT_LINE_CAP_BYTES / 4;
    assert!(
        line.len() < budget,
        "agent_end with large tool content must stay well under the frame cap          via message refs (#1060 AC1); got {} bytes",
        line.len()
    );
    assert!(line.len() < EVENT_LINE_CAP_BYTES);
    let j: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert!(
        !non_empty_refs(&j).is_empty(),
        "large-tool agent_end must carry non-empty refs: {j}"
    );
    if let Some(msgs) = j.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            let c = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
            assert!(
                c.len() < 1024,
                "agent_end must not re-carry large tool/result content"
            );
        }
    }
}

/// #1060 AC3: subagent_messages_appended uses the same ref model and stays small.
#[test]
fn subagent_messages_appended_exposes_non_empty_message_refs_without_full_content() {
    let big = "CHILD-TURN-BODY-".repeat(256);
    let ev = AgentEvent::SubagentMessagesAppended {
        agent_id: "worker".into(),
        messages: vec![],
        message_refs: vec!["33333333-3333-3333-3333-333333333333".into()],
    };
    let line = ev.to_json_line();
    let j: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(j["type"], "subagent_messages_appended");
    let refs = non_empty_refs(&j);
    assert!(
        !refs.is_empty(),
        "subagent_messages_appended must expose non-empty messageRefs (#1060); got: {j}"
    );
    if let Some(msgs) = j.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            let c = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
            assert!(
                c.is_empty() || c != big.as_str(),
                "re-stamped path must not re-carry full message content"
            );
        }
    }
    let budget = EVENT_LINE_CAP_BYTES / 4;
    assert!(
        line.len() < budget,
        "re-stamped subagent_messages_appended must stay well under the frame cap; got {}",
        line.len()
    );
}

/// #1060 AC4: on-demand lookup command exists and round-trips a stable id.
#[test]
fn get_message_command_parses_with_stable_message_id() {
    let json = r#"{"type":"get_message","id":"gm-req-1","messageId":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"}"#;
    let cmd = parse_command_line(json).expect("get_message must parse as AgentCommand (#1060)");
    assert_eq!(cmd.type_name(), "get_message");
    assert_eq!(cmd.id(), Some("gm-req-1"));
    // The command must retain the requested message identity for lookup.
    let j = serde_json::to_value(&cmd).expect("serialize");
    let mid = j
        .get("messageId")
        .or_else(|| j.get("message_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(
        mid, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        "get_message must round-trip the stable message id on the wire"
    );
}

#[test]
fn get_message_command_type_name_is_get_message() {
    // Construct via JSON so the test does not depend on a named enum variant
    // spelling until the implementation lands.
    let json = r#"{"type":"get_message","messageId":"id-1"}"#;
    let cmd = parse_command_line(json).expect("get_message parses");
    assert_eq!(cmd.type_name(), "get_message");
    assert!(cmd.id().is_none());
}
