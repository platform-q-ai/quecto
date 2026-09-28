//! The projection's failure reports, warnings and bounds (#2285 review):
//! a failed turn never reports the previous answer, the CLI's result is
//! followed over an assistant `error`, and nothing the projection keeps
//! grows with the stream except the conversation itself.

use serde_json::json;

use super::tests::{
    RT_REPORT_2, fold, init, result, roundtrip, text, tokens, tool_result, tool_use,
};
use super::*;
use crate::application::external_agent::dto::{
    ADMISSION_WARNING_CAPACITY, AUDIT_INPUT_PREVIEW_BYTES, BACKGROUND_JOB_CAPACITY,
    GUARDRAIL_AUDIT_CAPACITY, TurnWarning,
};
use crate::domain::external_agent::stream::{BackgroundTask, RateLimitStatus};
use crate::domain::external_agent::turn::{FailureKind, TurnFailure};

pub(super) fn error_result(terminal_reason: &str, error: &str, total: f64) -> ResultEvent {
    ResultEvent {
        is_error: Some(true),
        terminal_reason: Some(terminal_reason.into()),
        errors: vec![error.into()],
        total_cost_usd: Some(total),
        ..ResultEvent::default()
    }
}

// Mapping row: `result.result` → final report; an error result
// (`error_max_turns` …) has `errors[]` and no `result`.
#[test]
fn a_failed_turn_without_text_reports_its_failure_not_the_previous_answer() {
    let mut projector = roundtrip();
    let ends = fold(
        &mut projector,
        &[
            init(),
            ExternalAgentEvent::Result(error_result(
                "max_turns",
                "Reached maximum number of turns (5)",
                0.06,
            )),
        ],
    );
    let TurnEnd::Failed(failure) = &ends[0].end else {
        panic!("an error result fails the turn");
    };
    let report = projector.report().expect("a report");
    assert_ne!(report.content, RT_REPORT_2);
    assert_eq!(report.content, failure.describe());
    assert!(
        report
            .content
            .contains("Reached maximum number of turns (5)")
    );
    assert_eq!(report.failure.as_ref(), Some(failure));
    assert_eq!(
        report.message_ordinal, None,
        "this turn has no assistant text; the previous turn's answer is not its report"
    );
}

#[test]
fn a_failed_turns_report_points_only_at_its_own_text() {
    // rt, then a turn that only calls tools and ends in max_turns.
    let mut projector = roundtrip();
    fold(
        &mut projector,
        &[
            init(),
            tool_use("msg_x", "toolu_x", "Bash"),
            tool_result("toolu_x", json!("ok"), false),
            ExternalAgentEvent::Result(error_result(
                "max_turns",
                "Reached maximum number of turns (1)",
                0.06,
            )),
        ],
    );
    assert_eq!(projector.report().and_then(|r| r.message_ordinal), None);

    let ends = fold(
        &mut projector,
        &[
            init(),
            text("msg_y", "Trying once more."),
            ExternalAgentEvent::Result(error_result(
                "max_turns",
                "Reached maximum number of turns (1)",
                0.07,
            )),
        ],
    );
    assert!(!ends[0].end.is_completed());
    let own = projector
        .messages()
        .iter()
        .find(|m| m.api_message_id.as_deref() == Some("msg_y"))
        .map(|m| m.ordinal);
    assert_eq!(projector.report().and_then(|r| r.message_ordinal), own);
}

#[test]
fn a_failed_turn_with_text_reports_that_text_as_a_failure() {
    let mut projector = roundtrip();
    let mut failed = error_result("api_error", "overloaded", 0.06);
    failed.result_text = Some("API Error: 529".into());
    projector.apply(&ExternalAgentEvent::Result(failed));
    let report = projector.report().expect("a report");
    assert_eq!(report.content, "API Error: 529");
    assert!(report.failure.is_some());
}

#[test]
fn an_assistant_error_on_a_completed_result_is_a_warning() {
    let mut projector = Projector::new();
    let ends = fold(
        &mut projector,
        &[
            text("msg_w", "done"),
            ExternalAgentEvent::AssistantError {
                message_id: Some("msg_w".into()),
                kind: "rate_limit".into(),
            },
            ExternalAgentEvent::Result(result(
                "done",
                tokens(1, 1, 0, 0),
                0.01,
                tokens(1, 1, 0, 0),
            )),
        ],
    );
    assert_eq!(ends[0].end, TurnEnd::Completed);
    assert_eq!(
        ends[0].warnings,
        vec![TurnWarning::AssistantError("rate_limit".into())]
    );
    assert_eq!(projector.report().and_then(|r| r.failure.clone()), None);
}

#[test]
fn a_dropped_cumulative_cost_inside_a_process_charges_nothing_and_warns() {
    let mut projector = roundtrip();
    let ends = fold(
        &mut projector,
        &[ExternalAgentEvent::Result(result(
            "again",
            tokens(1, 1, 0, 0),
            0.0,
            tokens(1, 1, 0, 0),
        ))],
    );
    assert_eq!(ends[0].usage.cost_micro_usd, 0);
    assert_eq!(
        ends[0].warnings,
        vec![TurnWarning::CumulativeCostDropped {
            previous_micro_usd: 53_065,
            reported_micro_usd: 0,
        }]
    );
    assert_eq!(projector.session_totals().cost_micro_usd, 53_065);
}

#[test]
fn a_tool_result_without_an_id_closes_the_oldest_open_call() {
    let mut projector = Projector::new();
    fold(
        &mut projector,
        &[
            tool_use("msg_o", "toolu_a", "Bash"),
            tool_use("msg_o", "toolu_b", "Bash"),
            ExternalAgentEvent::ToolResult(ToolResultEvent {
                tool_use_id: None,
                content: json!("a done"),
                is_error: false,
                permission_denied: false,
            }),
        ],
    );
    assert_eq!(
        projector.messages()[1].tool_call_id.as_deref(),
        Some("toolu_a")
    );
    assert_eq!(projector.state(), ExecutionState::Thinking);
    projector.apply(&ExternalAgentEvent::ToolResult(ToolResultEvent {
        tool_use_id: None,
        content: json!("b done"),
        is_error: false,
        permission_denied: false,
    }));
    assert_eq!(
        projector.messages()[2].tool_call_id.as_deref(),
        Some("toolu_b")
    );
}

fn denial_turn(index: usize, input: serde_json::Value) -> ExternalAgentEvent {
    let mut denied = result(
        "refused",
        tokens(1, 1, 0, 0),
        0.001 * (index + 1) as f64,
        tokens(1, 1, 0, 0),
    );
    denied.permission_denials = vec![PermissionDenial {
        tool_name: "Bash".into(),
        tool_use_id: format!("toolu_{index}"),
        tool_input: input,
    }];
    ExternalAgentEvent::Result(denied)
}

#[test]
fn the_guardrail_audit_keeps_the_latest_denials_with_bounded_previews() {
    let mut projector = Projector::new();
    let turns = GUARDRAIL_AUDIT_CAPACITY + 8;
    for index in 0..turns {
        projector.apply(&denial_turn(index, json!({"command": "git push"})));
    }
    let audit: Vec<_> = projector.guardrail_audit().collect();
    assert_eq!(audit.len(), GUARDRAIL_AUDIT_CAPACITY);
    assert_eq!(audit[0].turn, 8, "the oldest are dropped first");
    assert_eq!(audit.last().map(|d| d.turn), Some(turns - 1));
    assert_eq!(projector.session_totals().guardrail_denials, turns);

    let huge = json!({"command": "é".repeat(4 * AUDIT_INPUT_PREVIEW_BYTES)});
    projector.apply(&denial_turn(turns, huge));
    let preview = &projector
        .guardrail_audit()
        .last()
        .expect("a denial")
        .tool_input_preview;
    assert!(
        preview.len() <= AUDIT_INPUT_PREVIEW_BYTES,
        "{}",
        preview.len()
    );
    assert!(preview.starts_with(r#"{"command":"é"#));
}

#[test]
fn admission_warnings_keep_the_latest_and_count_the_rest() {
    let mut projector = Projector::new();
    let total = ADMISSION_WARNING_CAPACITY + 4;
    for index in 0..total {
        projector.apply(&ExternalAgentEvent::RateLimit(RateLimitInfo {
            status: RateLimitStatus::AllowedWarning,
            limit_type: None,
            utilization: Some(index as f64 / 100.0),
            resets_at: None,
            windows: Vec::new(),
            using_overage: None,
        }));
    }
    let kept: Vec<_> = projector.admission_warnings().collect();
    assert_eq!(kept.len(), ADMISSION_WARNING_CAPACITY);
    assert_eq!(kept[0].utilization, Some(0.04));
    assert_eq!(projector.session_totals().admission_warnings, total);
    assert_eq!(
        projector.rate_limit().and_then(|r| r.utilization),
        Some((total - 1) as f64 / 100.0)
    );
}

fn started(task_id: &str, backgrounded: bool) -> ExternalAgentEvent {
    ExternalAgentEvent::TaskStarted(TaskStarted {
        task_id: task_id.into(),
        tool_use_id: None,
        description: None,
        is_backgrounded: backgrounded,
    })
}

// Mapping row: `background_tasks_changed` is the full current list.
#[test]
fn background_tasks_changed_replaces_the_background_set() {
    let mut projector = Projector::new();
    let listed = |ids: &[&str]| ExternalAgentEvent::BackgroundTasksChanged {
        tasks: ids
            .iter()
            .map(|id| BackgroundTask {
                task_id: id.to_string(),
                description: Some(format!("job {id}")),
            })
            .collect(),
    };
    fold(
        &mut projector,
        &[
            started("fg", false),
            listed(&["a", "b"]),
            started("a", true),
            listed(&["b"]),
        ],
    );
    let mut ids: Vec<_> = projector
        .background_jobs()
        .map(|j| j.task_id.clone())
        .collect();
    ids.sort();
    assert_eq!(ids, ["b", "fg"], "a left the list; a foreground task stays");
    let b = projector
        .background_jobs()
        .find(|j| j.task_id == "b")
        .expect("b is tracked");
    assert!(b.is_backgrounded);
    assert_eq!(b.description.as_deref(), Some("job b"));
}

#[test]
fn background_jobs_are_bounded() {
    let mut projector = Projector::new();
    for index in 0..BACKGROUND_JOB_CAPACITY + 6 {
        projector.apply(&started(&format!("t{index:03}"), false));
    }
    let jobs: Vec<_> = projector.background_jobs().collect();
    assert_eq!(jobs.len(), BACKGROUND_JOB_CAPACITY);
    assert!(
        jobs.iter().all(|j| j.task_id.as_str() >= "t006"),
        "the oldest go first"
    );
}

#[test]
fn a_failure_is_described_the_same_in_the_turn_end_and_the_report() {
    let mut projector = Projector::new();
    let ends = fold(
        &mut projector,
        &[ExternalAgentEvent::Result(error_result(
            "budget_exhausted",
            "Reached maximum budget ($0.5)",
            0.51,
        ))],
    );
    let expected = TurnFailure {
        terminal_reason: Some("budget_exhausted".into()),
        api_error_status: None,
        assistant_error: None,
        errors: vec!["Reached maximum budget ($0.5)".into()],
        kind: FailureKind::Error,
    };
    assert_eq!(ends[0].end, TurnEnd::Failed(expected.clone()));
    assert_eq!(
        projector.report().map(|r| r.content.clone()),
        Some(expected.describe())
    );
}
