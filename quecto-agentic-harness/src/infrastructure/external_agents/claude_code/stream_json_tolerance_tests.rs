//! One bad side field never loses an event (#2285 review): the turn-end
//! essentials of a `result` are decoded strictly, every other field on
//! its own with a default, and a result whose essentials are malformed is
//! still a (failed) turn end.

use serde_json::json;

use super::super::*;
use super::{decode, single};
use crate::domain::external_agent::stream::ResultEvent;
use crate::domain::external_agent::turn::TurnEnd;

fn result_of(line: serde_json::Value) -> ResultEvent {
    match single(line) {
        ExternalAgentEvent::Result(result) => result,
        other => panic!("a result line is always a Result, got {other:?}"),
    }
}

fn completed_with(side: serde_json::Value) -> serde_json::Value {
    let mut line = json!({
        "type": "result", "subtype": "success", "is_error": false,
        "terminal_reason": "completed", "result": "done", "total_cost_usd": 0.5,
    });
    for (key, value) in side.as_object().expect("an object") {
        line[key] = value.clone();
    }
    line
}

#[test]
fn a_bad_side_field_of_a_result_keeps_the_turn_end() {
    for side in [
        json!({"usage": null}),
        json!({"modelUsage": null}),
        json!({"permission_denials": null}),
        json!({"duration_ms": 12.5}),
        json!({"usage": {"input_tokens": -1, "output_tokens": 7}}),
        json!({"permission_denials": [{"tool_name": "Bash", "tool_input": {}}]}),
        json!({"num_turns": "four"}),
        json!({"stop_reason": 3}),
        json!({"errors": "not a list"}),
        json!({"modelUsage": {"m": null}}),
    ] {
        let result = result_of(completed_with(side.clone()));
        assert_eq!(
            TurnEnd::classify(&result, None),
            TurnEnd::Completed,
            "{side}"
        );
        assert_eq!(result.result_text.as_deref(), Some("done"), "{side}");
        assert_eq!(result.total_cost_usd, Some(0.5), "{side}");
    }
}

#[test]
fn a_side_field_is_decoded_as_far_as_it_is_well_formed() {
    let result = result_of(completed_with(json!({
        "duration_ms": 12.5,
        "usage": {"input_tokens": -1, "output_tokens": 7},
        "permission_denials": [{"tool_name": "Bash", "tool_input": {"command": "ls"}}],
    })));
    assert_eq!(result.duration_ms, Some(13));
    assert_eq!(result.usage.input, 0);
    assert_eq!(result.usage.output, 7);
    assert_eq!(result.permission_denials.len(), 1);
    assert_eq!(result.permission_denials[0].tool_name, "Bash");
    assert_eq!(result.permission_denials[0].tool_use_id, "");
}

#[test]
fn a_result_with_malformed_essentials_is_a_failed_turn_end() {
    for essentials in [
        json!({"is_error": "no"}),
        json!({"terminal_reason": 7}),
        json!({"result": {"text": "done"}}),
        json!({"total_cost_usd": "0.5"}),
    ] {
        let result = result_of(completed_with(essentials.clone()));
        let TurnEnd::Failed(failure) = TurnEnd::classify(&result, None) else {
            panic!("malformed essentials {essentials} must fail the turn");
        };
        assert!(
            failure.errors.iter().any(|e| e.contains("malformed")),
            "{essentials}: {:?}",
            failure.errors
        );
    }
}

// Mapping row: an error `result` (`error_max_turns`, `error_max_budget_usd`,
// `error_during_execution`) carries `errors[]` and no `result`.
#[test]
fn an_error_result_decodes_its_errors() {
    let result = result_of(json!({
        "type": "result", "subtype": "error_max_turns", "is_error": true,
        "terminal_reason": "max_turns", "errors": ["Reached maximum number of turns (5)", 7],
    }));
    assert_eq!(result.errors, vec!["Reached maximum number of turns (5)"]);
    assert_eq!(result.result_text, None);
}

#[test]
fn init_tolerates_bad_side_fields() {
    let ExternalAgentEvent::Init(init) = single(json!({
        "type": "system", "subtype": "init", "session_id": "s", "tools": null,
        "mcp_servers": [{"status": "connected"}, {"name": "board", "status": "failed"}, 3],
        "model": 4,
    })) else {
        panic!("an init");
    };
    assert_eq!(init.session_id.as_deref(), Some("s"));
    assert!(init.tools.is_empty());
    assert_eq!(init.model, None);
    assert_eq!(init.mcp_servers.len(), 2);
    assert_eq!(init.mcp_servers[0].name, "");
    assert!(!init.mcp_servers_connected());
}

#[test]
fn a_rate_limit_event_tolerates_float_timestamps() {
    let ExternalAgentEvent::RateLimit(info) = single(json!({
        "type": "rate_limit_event",
        "rate_limit_info": {"status": "allowed", "resetsAt": 1791072000.5,
            "unifiedWindows": {"five_hour": {"resetsAt": 1790617200.0, "utilization": "x"}}},
    })) else {
        panic!("a rate limit");
    };
    assert_eq!(info.status, RateLimitStatus::Allowed);
    assert_eq!(info.resets_at, Some(1_791_072_000));
    assert_eq!(info.windows[0].resets_at, Some(1_790_617_200));
    assert_eq!(info.windows[0].utilization, None);
}

#[test]
fn an_assistant_line_tolerates_null_content() {
    let events = decode(
        &json!({"type": "assistant", "message": {"id": "m", "content": null},
                "error": "authentication_failed"})
        .to_string(),
    )
    .expect("decodes");
    assert_eq!(
        events,
        vec![ExternalAgentEvent::AssistantError {
            message_id: Some("m".into()),
            kind: "authentication_failed".into(),
        }]
    );
}

#[test]
fn a_tool_result_with_bad_fields_still_closes_its_call() {
    let line = json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "content": "x", "is_error": null},
        {"type": "tool_result", "tool_use_id": "toolu_2", "is_error": "yes"},
    ]}});
    let events = decode(&line.to_string()).expect("decodes");
    let results: Vec<_> = events
        .iter()
        .map(|e| match e {
            ExternalAgentEvent::ToolResult(result) => result,
            other => panic!("a tool result, not {other:?}"),
        })
        .collect();
    assert_eq!(results[0].tool_use_id, None);
    assert!(!results[0].is_error);
    assert_eq!(results[1].tool_use_id.as_deref(), Some("toolu_2"));
    assert!(results[1].is_error, "a malformed is_error is an error");
}

#[test]
fn redacted_thinking_is_thinking_without_text() {
    let events = decode(
        &json!({"type": "assistant", "message": {"id": "m", "content": [
            {"type": "redacted_thinking", "data": "opaque"}
        ]}})
        .to_string(),
    )
    .expect("decodes");
    assert_eq!(
        events,
        vec![ExternalAgentEvent::AssistantBlock {
            message_id: "m".into(),
            block: AssistantContent::Thinking { text: None },
        }]
    );
}

#[test]
fn assistant_blocks_without_a_message_id_share_a_synthetic_one() {
    let mut decoder = StreamJsonDecoder::new();
    let line = json!({"type": "assistant", "message": {"content": [
        {"type": "text", "text": "a"}, {"type": "text", "text": "b"}
    ]}})
    .to_string();
    let first = decoder.decode_line(&line).expect("decodes");
    let second = decoder.decode_line(&line).expect("decodes");
    let ids = |events: &[ExternalAgentEvent]| -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                ExternalAgentEvent::AssistantBlock { message_id, .. } => message_id.clone(),
                other => panic!("a block, not {other:?}"),
            })
            .collect()
    };
    let (first, second) = (ids(&first), ids(&second));
    assert_eq!(first.len(), 2, "no text is lost");
    assert_eq!(first[0], first[1], "one line's blocks form one message");
    assert_ne!(first[0], second[0], "another line is another message");
}

#[test]
fn an_unknown_type_is_logged_once() {
    let mut decoder = StreamJsonDecoder::new();
    for _ in 0..3 {
        decoder
            .decode_line(r#"{"type": "stream_event"}"#)
            .expect("decodes");
    }
    decoder
        .decode_line(r#"{"type": "system", "subtype": "hook_started"}"#)
        .expect("decodes");
    assert_eq!(decoder.unknown_kinds_logged(), 2);
}

#[test]
fn a_task_event_without_its_task_id_is_unknown_not_an_error() {
    let mut decoder = StreamJsonDecoder::new();
    for _ in 0..2 {
        assert_eq!(
            decoder
                .decode_line(r#"{"type": "system", "subtype": "task_started", "description": "x"}"#)
                .expect("decodes"),
            vec![ExternalAgentEvent::Unknown {
                kind: "system/task_started".into()
            }]
        );
    }
    assert_eq!(
        decoder
            .decode_line(r#"{"type": "system", "subtype": "task_notification", "task_id": 7}"#)
            .expect("decodes"),
        vec![ExternalAgentEvent::Unknown {
            kind: "system/task_notification".into()
        }]
    );
    assert_eq!(decoder.unknown_kinds_logged(), 2, "logged once per kind");
}
