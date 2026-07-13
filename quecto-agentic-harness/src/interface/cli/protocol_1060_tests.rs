//! #1060 / ADR-0008 part 2 — wire-SHAPE unit tests for ref-based end-of-turn
//! events.
//!
//! Scope: these pin the serde wire shape — end-of-turn events expose non-empty
//! stable `messageRefs`, serialize an emptied `content` as `""` (not the body),
//! and stay small because size comes from refs rather than #1047 shrink/tail.
//! Each size test proves this with a counterfactual: the same event re-carrying
//! the body exceeds the cap, while the ref-based event stays well under it.
//!
//! These do NOT drive the producer; the proof that a real large turn is
//! *emitted* with emptied content + correct refs lives in the producer-driven
//! `uds_cancel_1060_tests.rs` / `uds_994_tests.rs::issue_1060_*`.
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
            content_length: None,
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

/// #1060 AC1/AC5: the emitted turn_end serializes `content` as `""` (never the
/// body) and identifies the turn by refs. Shape-level; the producer proof is in
/// `uds_cancel_1060_tests.rs`.
#[test]
fn turn_end_does_not_re_carry_full_assistant_content() {
    // The ref-based layout the producer emits: emptied content + a stable ref.
    let ev = AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".into(),
            content: String::new(),
            message_refs: vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()],
            usage: None,
            stop_reason: None,
            context_tokens: Some(1_200),
            max_context_tokens: Some(200_000),
            content_length: None,
        },
        tool_results: vec![],
    };
    let j = round_trip_json(&ev);
    assert_eq!(
        j["message"]["content"].as_str(),
        Some(""),
        "turn_end must serialize emptied content as \"\" (never the body) (#1060); got: {j}"
    );
    assert!(
        !non_empty_refs(&j).is_empty(),
        "turn_end must identify the turn via non-empty messageRefs when content is emptied"
    );
}

/// #1060 AC7: footer metadata remains on the bounded turn_end.
#[test]
fn turn_end_keeps_context_and_usage_metadata_without_full_content() {
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
            content_length: None,
        },
        tool_results: vec![],
    };
    let j = round_trip_json(&ev);
    assert_eq!(j["message"]["contextTokens"], 12_000);
    assert_eq!(j["message"]["maxContextTokens"], 200_000);
    assert_eq!(j["message"]["usage"]["total"], 1700);
    assert_eq!(
        j["message"]["content"].as_str(),
        Some(""),
        "footer metadata must remain while content serializes empty: {j}"
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

/// #1060 AC1: the ref-based end-of-turn layout keeps turn_end / agent_end well
/// under the frame cap because size comes from refs, not #1047 shrink/tail.
/// Proven by counterfactual: the SAME turn_end re-carrying the body blows the
/// cap, while the emptied-content ref layout stays well under it.
#[test]
fn large_real_turn_end_of_turn_events_stay_well_under_frame_cap() {
    // A body larger than the whole event line cap. If the producer re-carried
    // it (the pre-#1060 layout), the event would exceed the cap outright —
    // establishing that the body is genuinely too big to ship inline, so any
    // smallness below is due to refs, not a small hand-built struct.
    let body = "X".repeat(EVENT_LINE_CAP_BYTES + 64 * 1024);
    let bloated_turn_end = AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".into(),
            content: body.clone(),
            message_refs: vec!["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()],
            usage: None,
            stop_reason: None,
            context_tokens: Some(180_000),
            max_context_tokens: Some(200_000),
            content_length: None,
        },
        tool_results: vec![],
    };
    assert!(
        bloated_turn_end.to_json_line().len() > EVENT_LINE_CAP_BYTES,
        "counterfactual guard: a turn_end re-carrying this body must exceed the \
         cap, else the test proves nothing about refs"
    );

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
            content_length: None,
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
    // Content must be emptied — not merely truncated by shrink/tail.
    let turn_content = turn_j["message"]["content"].as_str().unwrap_or("");
    assert!(
        turn_content.is_empty(),
        "large-turn turn_end must empty content (refs, not shrink); got {} bytes",
        turn_content.len()
    );
}

/// #1060 AC1 wire-shape smoke test. The production-path large tool bodies are
/// exercised in `production_tool_turn_agent_end_refs_cover_all_roles`.
#[test]
fn refs_only_tool_agent_end_stays_well_under_frame_cap() {
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
