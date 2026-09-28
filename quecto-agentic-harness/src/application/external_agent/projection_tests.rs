//! The projection of spike #2264's streams (#2285). The event sequences
//! mirror the spike's captures (`tests/fixtures/claude_code/*.stream.jsonl`,
//! values copied); decoding the captures themselves is the codec's test.
//! Each mapping-table row of the spike report is named in a `Mapping row:`
//! line above the test that covers it.

use serde_json::json;

use super::*;
use crate::domain::external_agent::stream::{
    BackgroundTask, McpServerStatus, ModelUsage, RateLimitStatus, RateLimitWindow, TokenCounts,
};
use crate::domain::external_agent::turn::{FailureKind, TurnFailure};

const HAIKU: &str = "claude-haiku-4-5-20251001";

pub(super) fn init() -> ExternalAgentEvent {
    ExternalAgentEvent::Init(InitEvent {
        session_id: Some("00000000-0000-4000-8000-000000000001".into()),
        model: Some(HAIKU.into()),
        tools: vec!["Bash".into(), "mcp__board__board_claim".into()],
        mcp_servers: vec![McpServerStatus {
            name: "board".into(),
            status: "connected".into(),
        }],
        api_key_source: Some("none".into()),
        permission_mode: Some("bypassPermissions".into()),
    })
}

fn thinking_tokens() -> ExternalAgentEvent {
    ExternalAgentEvent::ThinkingTokens {
        estimated_tokens: Some(50),
    }
}

fn block(message_id: &str, block: AssistantContent) -> ExternalAgentEvent {
    ExternalAgentEvent::AssistantBlock {
        message_id: message_id.into(),
        block,
    }
}

fn thinking(message_id: &str) -> ExternalAgentEvent {
    block(message_id, AssistantContent::Thinking { text: None })
}

pub(super) fn text(message_id: &str, text: &str) -> ExternalAgentEvent {
    block(message_id, AssistantContent::Text(text.into()))
}

pub(super) fn tool_use(message_id: &str, id: &str, name: &str) -> ExternalAgentEvent {
    block(
        message_id,
        AssistantContent::ToolUse {
            id: id.into(),
            name: name.into(),
            input: json!({"command": "python3 hello.py"}),
        },
    )
}

pub(super) fn tool_result(
    id: &str,
    content: serde_json::Value,
    is_error: bool,
) -> ExternalAgentEvent {
    ExternalAgentEvent::ToolResult(ToolResultEvent {
        tool_use_id: Some(id.into()),
        content,
        is_error,
        permission_denied: false,
    })
}

pub(super) fn tokens(input: u64, output: u64, cache_read: u64, cache_write: u64) -> TokenCounts {
    TokenCounts {
        input,
        output,
        cache_read,
        cache_write,
    }
}

/// A completed turn's result with `rt`'s shape.
pub(super) fn result(
    text: &str,
    usage: TokenCounts,
    total: f64,
    cumulative: TokenCounts,
) -> ResultEvent {
    ResultEvent {
        is_error: Some(false),
        terminal_reason: Some("completed".into()),
        stop_reason: Some("end_turn".into()),
        api_error_status: None,
        result_text: Some(text.into()),
        errors: Vec::new(),
        usage,
        total_cost_usd: Some(total),
        model_usage: vec![ModelUsage {
            model: HAIKU.into(),
            tokens: cumulative,
            cost_usd: Some(total),
        }],
        permission_denials: Vec::new(),
        num_turns: Some(2),
        duration_ms: Some(4844),
        user_turn_ids: Vec::new(),
    }
}

const RT_REPORT_1: &str = "Done! Task T1 has been submitted.";
pub(super) const RT_REPORT_2: &str =
    "Done! I've replied to the coordinator.\n\n**The task id I submitted earlier was T1.**";

fn rt_turn_1() -> Vec<ExternalAgentEvent> {
    vec![
        init(),
        thinking_tokens(),
        thinking("msg_1"),
        text("msg_1", "I'll help you with the swarm task. "),
        tool_use("msg_1", "toolu_1", "mcp__board__board_summary"),
        tool_result(
            "toolu_1",
            json!([{"type": "text", "text": "{\"member\": \"W1\"}"}]),
            false,
        ),
        thinking("msg_2"),
        text("msg_2", "Now I'll run it."),
        tool_use("msg_2", "toolu_2", "Bash"),
        tool_result("toolu_2", json!("Hello, World!"), false),
        thinking("msg_3"),
        text("msg_3", RT_REPORT_1),
        ExternalAgentEvent::Result(result(
            RT_REPORT_1,
            tokens(58, 1069, 86550, 15403),
            0.044864,
            tokens(58, 1069, 86550, 15403),
        )),
    ]
}

fn rt_turn_2() -> Vec<ExternalAgentEvent> {
    vec![
        init(),
        thinking("msg_4"),
        tool_use("msg_4", "toolu_3", "mcp__board__board_inbox"),
        tool_result(
            "toolu_3",
            json!([{"type": "text", "text": "{\"messages\": []}"}]),
            false,
        ),
        thinking("msg_5"),
        text("msg_5", RT_REPORT_2),
        ExternalAgentEvent::Result(result(
            RT_REPORT_2,
            tokens(26, 362, 46907, 837),
            0.05306469999999999,
            tokens(84, 1431, 133457, 16240),
        )),
    ]
}

/// Fold `events`; the turn ends they gave.
pub(super) fn fold(projector: &mut Projector, events: &[ExternalAgentEvent]) -> Vec<TurnOutcome> {
    events
        .iter()
        .filter_map(|e| projector.apply(e).turn_end)
        .collect()
}

/// `rt`'s two turns: the projector and both turn ends.
fn roundtrip_turns() -> (Projector, Vec<TurnOutcome>) {
    let mut projector = Projector::new();
    projector.record_user_turn("Work the board: claim T1 and submit it.");
    let mut turns = fold(&mut projector, &rt_turn_1());
    projector.record_user_turn("What task id did you submit earlier?");
    turns.extend(fold(&mut projector, &rt_turn_2()));
    (projector, turns)
}

pub(super) fn roundtrip() -> Projector {
    roundtrip_turns().0
}

// Mapping row: `assistant` with a `text` block (group by `message.id`,
// the ordinal assigned on the first block).
#[test]
fn assistant_blocks_sharing_a_message_id_form_one_message() {
    let mut projector = Projector::new();
    projector.record_user_turn("go");
    fold(&mut projector, &rt_turn_1()[..6]);

    let assistant: Vec<_> = projector
        .messages()
        .iter()
        .filter(|m| m.role == MessageRole::Assistant)
        .collect();
    assert_eq!(assistant.len(), 1, "three events of msg_1 are one message");
    let message = assistant[0];
    assert_eq!(
        message.ordinal, 1,
        "ordinal of the first block, after the user turn"
    );
    assert_eq!(message.api_message_id.as_deref(), Some("msg_1"));
    assert_eq!(message.content, "I'll help you with the swarm task. ");
    assert_eq!(message.tool_calls.len(), 1);
    assert_eq!(projector.messages()[2].role, MessageRole::Tool);
    assert_eq!(projector.messages()[2].ordinal, 2);
}

#[test]
fn a_block_of_an_earlier_message_after_a_tool_result_keeps_that_message_ordinal() {
    // `rt` turn 2: msg_…CUJMCkiJrA5dikuv sends a tool_use, gets its result,
    // then a second tool_use under the same message id.
    let mut projector = Projector::new();
    fold(
        &mut projector,
        &[
            tool_use("msg_a", "toolu_a", "mcp__board__board_send"),
            tool_result("toolu_a", json!("{\"sent\": true}"), false),
            tool_use("msg_a", "toolu_b", "mcp__board__board_ack"),
        ],
    );
    let messages = projector.messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].tool_calls.len(), 2);
    assert_eq!(messages[1].ordinal, 1);
}

// Mapping row: `result.modelUsage[*]`, `total_cost_usd` (cumulative per
// process) → `SessionStats.costMicroUsd`.
#[test]
fn cost_is_the_delta_of_cumulative_total_cost() {
    let (projector, turns) = roundtrip_turns();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].usage.cost_micro_usd, 44_864);
    // 0.0530647 rounds once to 53_065 micro-USD; less 44_864.
    assert_eq!(turns[1].usage.cost_micro_usd, 8_201);
    assert_eq!(turns[1].usage.total_cost_micro_usd, 53_065);
    assert_eq!(projector.session_totals().cost_micro_usd, 53_065);
}

// Mapping row: `result.usage` → `TurnUsage {input, output}` (per turn);
// and `result.modelUsage[*]` → `SessionStats.tokens` (cumulative).
#[test]
fn turn_usage_is_per_turn_not_cumulative() {
    let (projector, turns) = roundtrip_turns();
    assert_eq!(turns[0].usage.tokens, tokens(58, 1069, 86550, 15403));
    assert_eq!(turns[1].usage.tokens, tokens(26, 362, 46907, 837));
    assert_eq!(
        projector.session_totals().tokens,
        tokens(84, 1431, 133457, 16240),
        "session tokens are modelUsage's cumulative counts, not a sum of turns"
    );
}

// Mapping rows: `user` with a `tool_result` block (the
// `tool_result_meta` "permission-rule" marks a hook denial) and
// `result.permission_denials[]` → guardrail audit.
#[test]
fn a_hook_denial_is_a_tool_error_and_an_audit_entry() {
    let reason =
        "PreToolUse:Bash hook error: quecto: command refused by the swarm denylist ('git push')";
    let mut denial_result = result(
        "quecto: command refused by the swarm denylist ('git push')",
        tokens(18, 203, 28582, 603),
        0.0204089,
        tokens(36, 676, 51049, 5944),
    );
    denial_result.permission_denials = vec![PermissionDenial {
        tool_name: "Bash".into(),
        tool_use_id: "toolu_g".into(),
        tool_input: json!({"command": "git push origin HEAD --dry-run"}),
    }];
    let mut projector = Projector::new();
    projector.record_user_turn("push it");
    let ends = fold(
        &mut projector,
        &[
            init(),
            thinking("msg_g"),
            tool_use("msg_g", "toolu_g", "Bash"),
            ExternalAgentEvent::ToolResult(ToolResultEvent {
                tool_use_id: Some("toolu_g".into()),
                content: json!(reason),
                is_error: true,
                permission_denied: true,
            }),
            thinking("msg_h"),
            text(
                "msg_h",
                "quecto: command refused by the swarm denylist ('git push')",
            ),
            ExternalAgentEvent::Result(denial_result),
        ],
    );

    let tool = projector
        .messages()
        .iter()
        .find(|m| m.role == MessageRole::Tool)
        .expect("the refusal is a tool message");
    assert!(tool.is_error);
    assert!(tool.permission_denied);
    assert_eq!(tool.tool_call_id.as_deref(), Some("toolu_g"));
    assert_eq!(tool.content, reason);
    assert_eq!(
        projector.guardrail_audit().cloned().collect::<Vec<_>>(),
        vec![GuardrailDenial {
            tool_name: "Bash".into(),
            tool_use_id: "toolu_g".into(),
            tool_input_preview: r#"{"command":"git push origin HEAD --dry-run"}"#.into(),
            turn: 0,
        }]
    );
    assert_eq!(projector.session_totals().guardrail_denials, 1);
    assert!(ends[0].end.is_completed(), "a refusal is not a failed turn");
}

#[test]
fn a_permission_denied_result_is_a_tool_error_even_unmarked() {
    let mut projector = Projector::new();
    projector.apply(&ExternalAgentEvent::ToolResult(ToolResultEvent {
        tool_use_id: Some("toolu_x".into()),
        content: json!("refused"),
        is_error: false,
        permission_denied: true,
    }));
    assert!(projector.messages()[0].is_error);
}

// Mapping row: `user` with a text block (the session records the turns it
// sends; a mid-turn wake folds into the running turn, like `steer`).
#[test]
fn a_mid_turn_wake_folds_into_one_result() {
    let mut projector = Projector::new();
    let first = projector.record_user_turn("Run: sleep 15 && echo slept");
    let events = [
        init(),
        thinking_tokens(),
        thinking("msg_m1"),
        tool_use("msg_m1", "toolu_m1", "Bash"),
        tool_result("toolu_m1", json!("slept"), false),
    ];
    let mut ends = fold(&mut projector, &events);
    let wake = projector.record_user_turn("You have a new board message.");
    ends.extend(fold(
        &mut projector,
        &[
            thinking("msg_m2"),
            text(
                "msg_m2",
                "Done sleeping.\n\nI also see there's a new board message.",
            ),
            tool_use("msg_m2", "toolu_m2", "mcp__board__board_inbox"),
            tool_result("toolu_m2", json!([{"type": "text", "text": "{}"}]), false),
            thinking("msg_m3"),
            text("msg_m3", "Done — I've acknowledged the board message."),
            ExternalAgentEvent::Result(result(
                "Done — I've acknowledged the board message.",
                tokens(34, 400, 50000, 900),
                0.0185118,
                tokens(34, 400, 50000, 900),
            )),
        ],
    ));

    assert_eq!(ends.len(), 1, "one result for both inputs");
    assert_eq!(projector.session_totals().turns, 1);
    let users: Vec<_> = projector
        .messages()
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .map(|m| (m.ordinal, m.content.as_str()))
        .collect();
    assert_eq!(
        users,
        vec![
            (first, "Run: sleep 15 && echo slept"),
            (wake, "You have a new board message."),
        ]
    );
    assert!(first < wake);
    assert_eq!(
        wake, 3,
        "after the first turn's assistant and tool messages"
    );
    assert_eq!(projector.session_totals().user_messages, 2);
}

#[test]
fn a_replayed_user_text_is_a_user_message() {
    let mut projector = Projector::new();
    projector.apply(&ExternalAgentEvent::UserText {
        text: "hello".into(),
    });
    assert_eq!(projector.messages()[0].role, MessageRole::User);
    assert_eq!(projector.messages()[0].content, "hello");
}

// Mapping row: `system/init` (re-emitted every turn) → model, sessionKey,
// `state=thinking`.
#[test]
fn init_repeated_each_turn_does_not_reset_state() {
    let mut projector = Projector::new();
    projector.record_user_turn("turn one");
    fold(&mut projector, &rt_turn_1());
    let messages_after_turn_1 = projector.messages().len();

    projector.apply(&init());

    assert_eq!(projector.messages().len(), messages_after_turn_1);
    assert_eq!(projector.session_totals().turns, 1);
    assert_eq!(
        projector.report().map(|r| r.content.as_str()),
        Some(RT_REPORT_1)
    );
    assert_eq!(projector.state(), ExecutionState::Thinking);
    let totals = projector.session_totals();
    assert_eq!(totals.cost_micro_usd, 44_864);
    assert_eq!(totals.model.as_deref(), Some(HAIKU));
    assert_eq!(
        totals.session_key.as_deref(),
        Some("00000000-0000-4000-8000-000000000001")
    );

    projector.record_user_turn("turn two");
    let turn_two = fold(&mut projector, &rt_turn_2()[1..]);
    let ordinals: Vec<u64> = projector.messages().iter().map(|m| m.ordinal).collect();
    let expected: Vec<u64> = (0..ordinals.len() as u64).collect();
    assert_eq!(ordinals, expected, "ordinals continue across turns");
    assert_eq!(turn_two[0].usage.cost_micro_usd, 8_201);
    assert_eq!(projector.last_turn(), turn_two.first());
}

// Mapping row: `result.result` → final report {messageId: last assistant
// text message, content, fullLengthBytes}, in 64 KiB pages.
#[test]
fn the_report_is_the_last_result_text() {
    let projector = roundtrip();
    let report = projector.report().expect("a report");
    assert_eq!(report.content, RT_REPORT_2);
    assert_eq!(report.full_length_bytes(), RT_REPORT_2.len());
    let last_text = projector
        .messages()
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::Assistant && !m.content.is_empty())
        .expect("an assistant message with text");
    assert_eq!(report.message_ordinal, Some(last_text.ordinal));
    assert_eq!(report.pages(), vec![RT_REPORT_2]);
    assert_eq!(report.failure, None);
}

#[test]
fn a_completed_result_without_text_does_not_report_the_previous_answer() {
    let mut projector = roundtrip();
    let mut bare = result("", TokenCounts::default(), 0.06, tokens(1, 1, 1, 1));
    bare.result_text = None;
    projector.apply(&ExternalAgentEvent::Result(bare));
    let report = projector.report().expect("a report");
    assert_eq!(report.content, "");
    assert_eq!(report.failure, None);
}

// Mapping rows: `system/thinking_tokens` → thinking; `assistant` `text` →
// streaming; `assistant` `tool_use` → runningTool; `user` `tool_result`
// → thinking; `result` → idle.
#[test]
fn the_stream_drives_the_execution_state() {
    let mut projector = Projector::new();
    assert_eq!(projector.state(), ExecutionState::Idle);
    let transitions: Vec<ExecutionState> = rt_turn_1()[..6]
        .iter()
        .filter_map(|event| projector.apply(event).transition)
        .collect();
    assert_eq!(
        transitions,
        [
            ExecutionState::Thinking,
            ExecutionState::Streaming,
            ExecutionState::RunningTool,
            ExecutionState::Thinking,
        ]
    );
    let events = rt_turn_1();
    let (last, middle) = events[6..].split_last().expect("a result");
    fold(&mut projector, middle);
    let step = projector.apply(last);
    assert_eq!(step.transition, Some(ExecutionState::Idle));
    assert!(step.turn_end.is_some());
    assert_eq!(
        projector.apply(&thinking_tokens()).transition,
        Some(ExecutionState::Thinking)
    );
    assert_eq!(
        projector.apply(&thinking_tokens()).transition,
        None,
        "no change, no transition"
    );
}

// Mapping row: `assistant` with a `tool_use` block → `message.toolCalls[]`
// (MCP tools are named `mcp__<server>__<tool>`).
#[test]
fn a_tool_use_block_is_a_tool_call_of_its_message() {
    let projector = roundtrip();
    let call = &projector.messages()[1].tool_calls[0];
    assert_eq!(call.id, "toolu_1");
    assert_eq!(call.name, "mcp__board__board_summary");
    assert_eq!(call.arguments, json!({"command": "python3 hello.py"}));
    assert_eq!(projector.session_totals().tool_calls, 3);
    assert_eq!(projector.session_totals().tool_results, 3);
}

// Mapping row: `assistant` with a `thinking` block → `message.thinking`
// (redacted: stays empty).
#[test]
fn a_redacted_thinking_block_leaves_thinking_empty() {
    let mut projector = Projector::new();
    projector.apply(&thinking("msg_t"));
    assert_eq!(projector.messages()[0].thinking, None);
    projector.apply(&block(
        "msg_t",
        AssistantContent::Thinking {
            text: Some("plain".into()),
        },
    ));
    assert_eq!(projector.messages()[0].thinking.as_deref(), Some("plain"));
}

// Mapping row: `system/task_started`, `task_notification`,
// `background_tasks_changed` → background-job tracking.
#[test]
fn task_events_track_background_jobs() {
    let mut projector = Projector::new();
    fold(
        &mut projector,
        &[
            ExternalAgentEvent::BackgroundTasksChanged {
                tasks: vec![BackgroundTask {
                    task_id: "bup00mo5i".into(),
                    description: Some("sleep 300".into()),
                }],
            },
            ExternalAgentEvent::TaskStarted(TaskStarted {
                task_id: "bup00mo5i".into(),
                tool_use_id: Some("toolu_k".into()),
                description: Some("sleep 300".into()),
                is_backgrounded: true,
            }),
            ExternalAgentEvent::TaskStarted(TaskStarted {
                task_id: "bu4k0uznb".into(),
                tool_use_id: Some("toolu_m".into()),
                description: Some("sleep 15 && echo slept".into()),
                is_backgrounded: false,
            }),
            ExternalAgentEvent::TaskNotification(TaskNotification {
                task_id: "bu4k0uznb".into(),
                tool_use_id: Some("toolu_m".into()),
                status: Some("completed".into()),
                summary: Some("sleep 15 && echo slept".into()),
            }),
        ],
    );
    let jobs: Vec<_> = projector.background_jobs().collect();
    assert_eq!(jobs.len(), 2);
    let finished = jobs.iter().find(|j| j.task_id == "bu4k0uznb").expect("job");
    assert_eq!(finished.status.as_deref(), Some("completed"));
    let running = jobs.iter().find(|j| j.task_id == "bup00mo5i").expect("job");
    assert!(running.is_backgrounded);
    assert_eq!(running.status, None);
    assert_eq!(running.tool_use_id.as_deref(), Some("toolu_k"));
}

// Mapping row: `rate_limit_event.rate_limit_info` → admission warnings.
#[test]
fn a_rate_limit_warning_is_an_admission_warning() {
    let warning = RateLimitInfo {
        status: RateLimitStatus::AllowedWarning,
        limit_type: Some("seven_day".into()),
        utilization: Some(0.62),
        resets_at: Some(1_791_072_000),
        windows: vec![RateLimitWindow {
            name: "five_hour".into(),
            utilization: Some(0.01),
            resets_at: Some(1_790_617_200),
        }],
        using_overage: Some(false),
    };
    let allowed = RateLimitInfo {
        status: RateLimitStatus::Allowed,
        ..warning.clone()
    };
    let mut projector = Projector::new();
    projector.apply(&ExternalAgentEvent::RateLimit(allowed.clone()));
    assert_eq!(projector.admission_warnings().count(), 0);
    projector.apply(&ExternalAgentEvent::RateLimit(warning.clone()));
    projector.apply(&ExternalAgentEvent::RateLimit(allowed.clone()));
    assert_eq!(
        projector.admission_warnings().cloned().collect::<Vec<_>>(),
        vec![warning]
    );
    assert_eq!(projector.session_totals().admission_warnings, 1);
    assert_eq!(projector.rate_limit(), Some(&allowed));
}

// Mapping rows: `result.stop_reason` → `StopReason`; `result.is_error`,
// `terminal_reason`, `api_error_status`, assistant `error` → end
// classification.
#[test]
fn a_not_logged_in_turn_ends_failed_with_every_signal() {
    let mut projector = Projector::new();
    let not_logged_in = ResultEvent {
        is_error: Some(true),
        terminal_reason: Some("api_error".into()),
        stop_reason: Some("stop_sequence".into()),
        result_text: Some("Not logged in · Please run /login".into()),
        ..ResultEvent::default()
    };
    let ends = fold(
        &mut projector,
        &[
            init(),
            text("msg_e", "Not logged in · Please run /login"),
            ExternalAgentEvent::AssistantError {
                message_id: Some("msg_e".into()),
                kind: "authentication_failed".into(),
            },
            ExternalAgentEvent::Result(not_logged_in),
        ],
    );
    assert_eq!(
        ends[0].end,
        TurnEnd::Failed(TurnFailure {
            terminal_reason: Some("api_error".into()),
            api_error_status: None,
            assistant_error: Some("authentication_failed".into()),
            errors: Vec::new(),
            kind: FailureKind::Error,
        })
    );
    assert_eq!(ends[0].stop_reason, Some(StopReason::EndTurn));
    let report = projector.report().expect("a report");
    assert_eq!(report.content, "Not logged in · Please run /login");
    assert!(report.failure.is_some(), "the report says the turn failed");
    assert_eq!(ends[0].usage.cost_micro_usd, 0);
    assert_eq!(projector.state(), ExecutionState::Idle);
}

#[test]
fn an_assistant_error_belongs_to_its_own_turn_only() {
    let mut projector = Projector::new();
    projector.apply(&ExternalAgentEvent::AssistantError {
        message_id: None,
        kind: "authentication_failed".into(),
    });
    projector.apply(&ExternalAgentEvent::Result(ResultEvent {
        is_error: Some(true),
        terminal_reason: Some("api_error".into()),
        ..ResultEvent::default()
    }));
    let outcome = projector
        .apply(&ExternalAgentEvent::Result(result(
            "ok",
            TokenCounts::default(),
            0.01,
            tokens(1, 1, 0, 0),
        )))
        .turn_end
        .expect("a result ends a turn");
    assert_eq!(outcome.end, TurnEnd::Completed);
    assert_eq!(outcome.stop_reason, Some(StopReason::EndTurn));
    assert_eq!(outcome.warnings, Vec::new());
}

#[test]
fn an_unknown_event_is_counted_and_changes_nothing_else() {
    let mut projector = roundtrip();
    let before = projector.messages().len();
    assert_eq!(
        projector.apply(&ExternalAgentEvent::Unknown {
            kind: "system/hook_started".into()
        }),
        ProjectionStep::default()
    );
    assert_eq!(projector.session_totals().unknown_events, 1);
    assert_eq!(projector.messages().len(), before);
    assert_eq!(projector.state(), ExecutionState::Idle);
}
