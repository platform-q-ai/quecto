//! Decoding spike #2264's real captures (#2285), scrubbed copies in
//! `tests/fixtures/claude_code/`.

use serde_json::json;

use super::*;
use crate::domain::external_agent::stream::{
    ModelUsage, PermissionDenial, ResultEvent, TokenCounts,
};

/// Decode one line with a fresh decoder.
pub(super) fn decode(line: &str) -> Result<Vec<ExternalAgentEvent>, StreamJsonError> {
    StreamJsonDecoder::new().decode_line(line)
}

pub(super) const SAMPLES: &[(&str, &str)] = &[
    (
        "rt",
        include_str!("../../../../tests/fixtures/claude_code/rt.stream.jsonl"),
    ),
    (
        "guards",
        include_str!("../../../../tests/fixtures/claude_code/guards.stream.jsonl"),
    ),
    (
        "mid",
        include_str!("../../../../tests/fixtures/claude_code/mid.stream.jsonl"),
    ),
    (
        "kill",
        include_str!("../../../../tests/fixtures/claude_code/kill.stream.jsonl"),
    ),
    // SYNTHESIZED (see tests/fixtures/claude_code/README.md), not captured:
    // from claude 2.1.280's result schema (`error_max_turns`,
    // `error_max_budget_usd`, `error_during_execution`), not captured.
    (
        "errors",
        include_str!("../../../../tests/fixtures/claude_code/errors.stream.jsonl"),
    ),
];

/// Event types the samples may carry that this vocabulary leaves
/// `Unknown` on purpose. None today: every captured type is typed.
const LISTED_UNKNOWN: &[&str] = &[];

/// Every event of a sample, decoded by one decoder.
pub(super) fn sample(name: &str) -> Vec<ExternalAgentEvent> {
    let (_, text) = SAMPLES
        .iter()
        .find(|(sample, _)| *sample == name)
        .expect("a known sample");
    let mut decoder = StreamJsonDecoder::new();
    text.lines()
        .flat_map(|line| {
            decoder
                .decode_line(line)
                .expect("every sample line decodes")
        })
        .collect()
}

pub(super) fn single(line: serde_json::Value) -> ExternalAgentEvent {
    let mut events = decode(&line.to_string()).expect("decodes");
    assert_eq!(events.len(), 1, "{events:?}");
    events.remove(0)
}

#[test]
fn every_line_of_every_spike_sample_decodes() {
    let mut lines = 0;
    for (name, text) in SAMPLES {
        for (index, line) in text.lines().enumerate() {
            lines += 1;
            let events = decode(line)
                .unwrap_or_else(|err| panic!("{name}:{} does not decode: {err}", index + 1));
            assert!(
                !events.is_empty(),
                "{name}:{} decodes to nothing",
                index + 1
            );
            for event in events {
                if let ExternalAgentEvent::Unknown { kind } = &event {
                    assert!(
                        LISTED_UNKNOWN.contains(&kind.as_str()),
                        "{name}:{} is an unlisted Unknown `{kind}`",
                        index + 1
                    );
                }
            }
        }
    }
    assert_eq!(lines, 43 + 16 + 18 + 11 + 7);
}

// Mapping row: `system/init` {session_id, model, tools, mcp_servers,
// permissionMode}.
#[test]
fn init_carries_the_session_configuration() {
    let events = sample("rt");
    let ExternalAgentEvent::Init(init) = &events[0] else {
        panic!("rt opens with init: {:?}", events[0]);
    };
    assert_eq!(
        init.session_id.as_deref(),
        Some("00000000-0000-4000-8000-000000000001")
    );
    assert_eq!(init.model.as_deref(), Some("claude-haiku-4-5-20251001"));
    assert_eq!(init.api_key_source.as_deref(), Some("none"));
    assert_eq!(init.permission_mode.as_deref(), Some("bypassPermissions"));
    assert_eq!(init.tools.len(), 13);
    assert!(init.tools_are_exactly(&[
        "Bash",
        "Edit",
        "Glob",
        "Grep",
        "Read",
        "Write",
        "mcp__board__board_ack",
        "mcp__board__board_claim",
        "mcp__board__board_inbox",
        "mcp__board__board_reserve",
        "mcp__board__board_send",
        "mcp__board__board_submit",
        "mcp__board__board_summary",
    ]));
    assert_eq!(
        init.mcp_servers,
        vec![McpServerStatus {
            name: "board".into(),
            status: "connected".into()
        }]
    );
    let inits = events
        .iter()
        .filter(|e| matches!(e, ExternalAgentEvent::Init(_)))
        .count();
    assert_eq!(inits, 2, "init repeats every turn");
}

// Mapping rows: `system/thinking_tokens`; `assistant` `thinking` (redacted),
// `text` and `tool_use` blocks sharing `message.id`.
#[test]
fn assistant_lines_decode_to_blocks_of_their_message() {
    let events = sample("rt");
    assert_eq!(
        events[1],
        ExternalAgentEvent::ThinkingTokens {
            estimated_tokens: Some(50)
        }
    );
    let id = "msg_011CfVtx5z5wLLGSaL7udoby";
    assert_eq!(
        events[2],
        ExternalAgentEvent::AssistantBlock {
            message_id: id.into(),
            block: AssistantContent::Thinking { text: None },
        }
    );
    assert!(matches!(
        &events[3],
        ExternalAgentEvent::AssistantBlock { message_id, block: AssistantContent::Text(text) }
            if message_id == id && text.starts_with("I'll help you")
    ));
    assert_eq!(
        events[4],
        ExternalAgentEvent::AssistantBlock {
            message_id: id.into(),
            block: AssistantContent::ToolUse {
                id: "toolu_01CfXHCEYemxMEfoRTbiPtCQ".into(),
                name: "mcp__board__board_summary".into(),
                input: json!({}),
            },
        }
    );
}

// Mapping row: `user` with a `tool_result` block {tool_use_id, content,
// is_error}; `tool_result_meta.non_execution_kind: "permission-rule"`
// marks a hook denial.
#[test]
fn a_hook_refusal_decodes_as_a_denied_error_result() {
    let events = sample("guards");
    let results: Vec<&ToolResultEvent> = events
        .iter()
        .filter_map(|e| match e {
            ExternalAgentEvent::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 2);
    for result in &results {
        assert!(result.is_error);
        assert!(result.permission_denied);
        assert!(result.content_text().starts_with("PreToolUse:"));
    }
    let ordinary = sample("rt");
    let ran = ordinary
        .iter()
        .find_map(|e| match e {
            ExternalAgentEvent::ToolResult(r)
                if r.tool_use_id.as_deref() == Some("toolu_01UMUaeXRrMxrLbPpWRpnq4e") =>
            {
                Some(r)
            }
            _ => None,
        })
        .expect("the Bash result");
    assert!(!ran.is_error);
    assert!(!ran.permission_denied);
    assert_eq!(ran.content, json!("Hello, World!"));
}

#[test]
fn a_meta_of_another_kind_is_not_a_denial() {
    let line = json!({
        "type": "user",
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "x", "is_error": true}
        ]},
        "tool_result_meta": [{"id": "toolu_1", "non_execution_kind": "cancelled"}]
    });
    let ExternalAgentEvent::ToolResult(result) = single(line) else {
        panic!("a tool result");
    };
    assert!(result.is_error);
    assert!(!result.permission_denied);
}

// Mapping row: `user` with a text block (only with
// `--replay-user-messages`).
#[test]
fn a_replayed_user_message_decodes_as_user_text() {
    let listed = json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "text", "text": "claim T1"}
    ]}});
    assert_eq!(
        single(listed),
        ExternalAgentEvent::UserText {
            text: "claim T1".into()
        }
    );
    let plain = json!({"type": "user", "message": {"role": "user", "content": "claim T2"}});
    assert_eq!(
        single(plain),
        ExternalAgentEvent::UserText {
            text: "claim T2".into()
        }
    );
}

// Mapping row: `system/task_started`, `task_notification`,
// `background_tasks_changed`.
#[test]
fn task_events_decode() {
    let kill = sample("kill");
    assert!(kill.contains(&ExternalAgentEvent::BackgroundTasksChanged {
        tasks: vec![BackgroundTask {
            task_id: "bup00mo5i".into(),
            description: Some("sleep 300".into()),
        }],
    }));
    assert!(kill.contains(&ExternalAgentEvent::TaskStarted(TaskStarted {
        task_id: "bup00mo5i".into(),
        tool_use_id: Some("toolu_013ubJiybt9Df4AMemMhZBhU".into()),
        description: Some("sleep 300".into()),
        is_backgrounded: true,
    })));
    let mid = sample("mid");
    assert!(
        mid.contains(&ExternalAgentEvent::TaskNotification(TaskNotification {
            task_id: "bu4k0uznb".into(),
            tool_use_id: Some("toolu_01YYpGPpPNniV3ZEJJJXhjdj".into()),
            status: Some("completed".into()),
            summary: Some("sleep 15 && echo slept".into()),
        }))
    );
}

// Mapping row: `rate_limit_event.rate_limit_info` {status, utilization per
// window, resetsAt, isUsingOverage}.
#[test]
fn a_rate_limit_event_decodes_every_window() {
    let rate = sample("rt")
        .into_iter()
        .find_map(|e| match e {
            ExternalAgentEvent::RateLimit(info) => Some(info),
            _ => None,
        })
        .expect("rt has a rate-limit event");
    assert_eq!(rate.status, RateLimitStatus::AllowedWarning);
    assert_eq!(rate.limit_type.as_deref(), Some("seven_day"));
    assert_eq!(rate.utilization, Some(0.62));
    assert_eq!(rate.resets_at, Some(1_791_072_000));
    assert_eq!(rate.using_overage, Some(false));
    assert_eq!(
        rate.windows,
        vec![
            RateLimitWindow {
                name: "five_hour".into(),
                utilization: Some(0.01),
                resets_at: Some(1_790_617_200),
            },
            RateLimitWindow {
                name: "seven_day".into(),
                utilization: Some(0.62),
                resets_at: Some(1_791_072_000),
            },
        ]
    );
}

fn results(name: &str) -> Vec<ResultEvent> {
    sample(name)
        .into_iter()
        .filter_map(|e| match e {
            ExternalAgentEvent::Result(result) => Some(result),
            _ => None,
        })
        .collect()
}

// Mapping rows: `result.result`, `result.usage` (per turn),
// `result.modelUsage[*]` and `total_cost_usd` (cumulative),
// `result.stop_reason`, `result.is_error` / `terminal_reason` /
// `api_error_status`.
#[test]
fn a_result_decodes_per_turn_and_cumulative_usage() {
    let rt = results("rt");
    assert_eq!(rt.len(), 2);
    let second = &rt[1];
    assert_eq!(second.is_error, Some(false));
    assert_eq!(second.terminal_reason.as_deref(), Some("completed"));
    assert_eq!(second.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(second.api_error_status, None);
    assert_eq!(second.num_turns, Some(4));
    assert_eq!(second.duration_ms, Some(4844));
    assert!(
        second
            .result_text
            .as_deref()
            .is_some_and(|t| t.ends_with("was T1.**"))
    );
    assert_eq!(
        second.usage,
        TokenCounts {
            input: 26,
            output: 362,
            cache_read: 46_907,
            cache_write: 837,
        }
    );
    assert_eq!(rt[0].total_cost_usd, Some(0.044864));
    assert_eq!(second.total_cost_usd, Some(0.05306469999999999));
    assert_eq!(
        second.model_usage,
        vec![ModelUsage {
            model: "claude-haiku-4-5-20251001".into(),
            tokens: TokenCounts {
                input: 84,
                output: 1431,
                cache_read: 133_457,
                cache_write: 16_240,
            },
            cost_usd: Some(0.05306469999999999),
        }]
    );
}

// Mapping row: `result.permission_denials[]`.
#[test]
fn a_result_decodes_its_permission_denials() {
    let guards = results("guards");
    assert_eq!(
        guards[1].permission_denials,
        vec![PermissionDenial {
            tool_name: "Bash".into(),
            tool_use_id: "toolu_01C1BieTLDzgUouWDEZRiNff".into(),
            tool_input: json!({"command": "git push origin HEAD --dry-run"}),
        }]
    );
    assert_eq!(guards[0].permission_denials[0].tool_name, "Write");
}

#[test]
fn the_not_logged_in_shape_decodes_its_error_signals() {
    let assistant = json!({
        "type": "assistant",
        "message": {"id": "msg_e", "role": "assistant", "content": [
            {"type": "text", "text": "Not logged in · Please run /login"}
        ]},
        "error": "authentication_failed"
    });
    let events = decode(&assistant.to_string()).expect("decodes");
    assert_eq!(
        events,
        vec![
            ExternalAgentEvent::AssistantBlock {
                message_id: "msg_e".into(),
                block: AssistantContent::Text("Not logged in · Please run /login".into()),
            },
            ExternalAgentEvent::AssistantError {
                message_id: Some("msg_e".into()),
                kind: "authentication_failed".into(),
            },
        ]
    );
    let result = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "terminal_reason": "api_error", "api_error_status": 401,
        "result": "Not logged in · Please run /login"
    });
    let ExternalAgentEvent::Result(result) = single(result) else {
        panic!("a result");
    };
    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.terminal_reason.as_deref(), Some("api_error"));
    assert_eq!(result.api_error_status, Some(401));
}

#[test]
fn a_missing_optional_field_is_tolerated() {
    let ExternalAgentEvent::Result(bare) = single(json!({"type": "result"})) else {
        panic!("a result");
    };
    assert_eq!(bare, ResultEvent::default());
    let nulls =
        json!({"type": "result", "usage": {"input_tokens": null}, "api_error_status": "503"});
    let ExternalAgentEvent::Result(nulls) = single(nulls) else {
        panic!("a result");
    };
    assert_eq!(nulls.usage, TokenCounts::default());
    assert_eq!(nulls.api_error_status, Some(503));
    assert_eq!(
        single(json!({"type": "system", "subtype": "init"})),
        ExternalAgentEvent::Init(InitEvent::default())
    );
    assert!(matches!(
        single(json!({"type": "rate_limit_event", "rate_limit_info": {}})),
        ExternalAgentEvent::RateLimit(RateLimitInfo {
            status: RateLimitStatus::Other(_),
            ..
        })
    ));
}

#[test]
fn an_unknown_type_is_unknown_not_a_panic() {
    assert_eq!(
        single(json!({"type": "stream_event", "event": {}})),
        ExternalAgentEvent::Unknown {
            kind: "stream_event".into()
        }
    );
    assert_eq!(
        single(json!({"type": "system", "subtype": "hook_started"})),
        ExternalAgentEvent::Unknown {
            kind: "system/hook_started".into()
        }
    );
    assert_eq!(
        single(json!({"no_type": true})),
        ExternalAgentEvent::Unknown {
            kind: String::new()
        }
    );
    let odd_block = json!({"type": "assistant", "message": {"id": "m", "content": [
        {"type": "server_tool_use", "id": "s"}
    ]}});
    assert_eq!(
        single(odd_block),
        ExternalAgentEvent::Unknown {
            kind: "assistant/server_tool_use".into()
        }
    );
}

#[test]
fn a_line_that_is_not_a_json_object_is_an_error() {
    assert!(matches!(
        decode("not json"),
        Err(StreamJsonError::NotJson(_))
    ));
    assert_eq!(decode("[1, 2]"), Err(StreamJsonError::NotAnObject));
}

#[cfg(test)]
#[path = "stream_json_projection_tests.rs"]
mod projection;
#[cfg(test)]
#[path = "stream_json_tolerance_tests.rs"]
mod tolerance;
