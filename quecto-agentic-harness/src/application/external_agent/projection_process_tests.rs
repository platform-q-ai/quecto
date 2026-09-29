//! A new agent process, finished jobs and large tool results (#2285
//! review round 2): costs are charged per process and summed, a restart
//! drops the old process's turn state, jobs evict finished ones first, and
//! a tool result is stored bounded.

use serde_json::json;

use super::tests::{fold, init, result, roundtrip, text, tokens, tool_result, tool_use};
use super::*;
use crate::application::external_agent::dto::{
    BACKGROUND_JOB_CAPACITY, TERMINAL_TASK_STATUSES, TOOL_RESULT_CONTENT_BYTES, TRUNCATION_MARKER,
};

#[test]
fn a_new_process_is_charged_its_whole_first_total() {
    let mut projector = roundtrip();
    projector.process_started();
    let ends = fold(
        &mut projector,
        &[
            init(),
            ExternalAgentEvent::Result(result("hi", tokens(1, 1, 0, 0), 0.06, tokens(1, 1, 0, 0))),
        ],
    );
    assert_eq!(ends[0].usage.cost_micro_usd, 60_000);
    assert_eq!(ends[0].warnings, Vec::new());
    assert_eq!(projector.session_totals().cost_micro_usd, 113_065);
}

#[test]
fn a_zero_total_then_a_higher_one_in_one_process_is_not_double_charged() {
    let mut projector = roundtrip();
    let zero = result("crashed", tokens(0, 0, 0, 0), 0.0, tokens(0, 0, 0, 0));
    let later = result("ok", tokens(1, 1, 0, 0), 0.06, tokens(1, 1, 0, 0));
    let ends = fold(
        &mut projector,
        &[
            ExternalAgentEvent::Result(zero),
            ExternalAgentEvent::Result(later),
        ],
    );
    assert_eq!(ends[0].usage.cost_micro_usd, 0);
    assert_eq!(ends[1].usage.cost_micro_usd, 6_935);
    assert_eq!(projector.session_totals().cost_micro_usd, 60_000);
}

#[test]
fn a_new_process_drops_the_old_process_turn_state() {
    let mut projector = Projector::new();
    fold(
        &mut projector,
        &[
            init(),
            text("msg_1", "half an answer"),
            tool_use("msg_1", "toolu_old", "Bash"),
            ExternalAgentEvent::AssistantError {
                message_id: Some("msg_1".into()),
                kind: "authentication_failed".into(),
            },
        ],
    );
    projector.process_started();
    assert_eq!(projector.state(), ExecutionState::Idle);
    let ends = fold(
        &mut projector,
        &[
            init(),
            text("msg_1", "a new answer"),
            ExternalAgentEvent::ToolResult(ToolResultEvent {
                tool_use_id: None,
                content: json!("orphan"),
                is_error: false,
                permission_denied: false,
            }),
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
        Vec::new(),
        "the stale assistant error is gone"
    );
    let answers: Vec<_> = projector
        .messages()
        .iter()
        .filter(|m| m.api_message_id.as_deref() == Some("msg_1"))
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(
        answers,
        ["half an answer", "a new answer"],
        "a new process's message"
    );
    let orphan = projector
        .messages()
        .iter()
        .find(|m| m.content == "orphan")
        .expect("the tool result");
    assert_eq!(
        orphan.tool_call_id, None,
        "the old process's call is not closed by it"
    );
}

fn started(task_id: &str) -> ExternalAgentEvent {
    ExternalAgentEvent::TaskStarted(TaskStarted {
        task_id: task_id.into(),
        tool_use_id: None,
        description: None,
        is_backgrounded: true,
    })
}

fn finished(task_id: &str, status: &str) -> ExternalAgentEvent {
    ExternalAgentEvent::TaskNotification(TaskNotification {
        task_id: task_id.into(),
        tool_use_id: None,
        status: Some(status.into()),
        summary: None,
    })
}

#[test]
fn a_terminal_notification_finishes_a_job() {
    let mut projector = Projector::new();
    for status in TERMINAL_TASK_STATUSES {
        projector.apply(&started(status));
        projector.apply(&finished(status, status));
    }
    projector.apply(&started("running"));
    projector.apply(&finished("running", "progress"));
    let finished: Vec<_> = projector
        .background_jobs()
        .filter(|job| job.is_finished())
        .map(|job| job.task_id.as_str())
        .collect();
    assert_eq!(finished, TERMINAL_TASK_STATUSES);
}

#[test]
fn a_full_job_list_evicts_finished_jobs_before_running_ones() {
    let mut projector = Projector::new();
    for index in 0..10 {
        let id = format!("done{index}");
        projector.apply(&started(&id));
        projector.apply(&finished(&id, "completed"));
    }
    let running = BACKGROUND_JOB_CAPACITY - 4;
    for index in 0..running {
        projector.apply(&started(&format!("run{index:02}")));
    }
    let jobs: Vec<_> = projector.background_jobs().collect();
    assert_eq!(jobs.len(), BACKGROUND_JOB_CAPACITY);
    assert_eq!(
        jobs.iter().filter(|job| !job.is_finished()).count(),
        running,
        "no running job is evicted while finished ones remain"
    );
    let kept_finished: Vec<_> = jobs
        .iter()
        .filter(|job| job.is_finished())
        .map(|job| job.task_id.as_str())
        .collect();
    assert_eq!(
        kept_finished,
        ["done6", "done7", "done8", "done9"],
        "the oldest finished go first"
    );
}

#[test]
fn a_large_tool_result_is_stored_bounded_with_its_length() {
    let big = "é".repeat(TOOL_RESULT_CONTENT_BYTES); // 2 bytes each
    let mut projector = Projector::new();
    fold(
        &mut projector,
        &[
            tool_use("msg_b", "toolu_b", "Read"),
            tool_result("toolu_b", json!(big), false),
            tool_result("toolu_small", json!("small"), false),
        ],
    );
    let stored = &projector.messages()[1];
    assert!(stored.content.ends_with(TRUNCATION_MARKER));
    assert!(stored.content.len() <= TOOL_RESULT_CONTENT_BYTES + TRUNCATION_MARKER.len());
    assert_eq!(stored.truncated_from_bytes, Some(big.len()));
    let small = &projector.messages()[2];
    assert_eq!(small.content, "small");
    assert_eq!(small.truncated_from_bytes, None);
}

#[test]
fn a_restarted_process_does_not_show_the_killed_process_jobs_running() {
    // kill.stream.jsonl: a background `sleep 300` never finishes before
    // the process is killed; the next process must not show it running.
    let mut projector = Projector::new();
    fold(
        &mut projector,
        &[
            ExternalAgentEvent::BackgroundTasksChanged {
                tasks: vec![crate::domain::external_agent::stream::BackgroundTask {
                    task_id: "bup00mo5i".into(),
                    description: Some("sleep 300".into()),
                }],
            },
            started("bup00mo5i"),
        ],
    );
    projector.process_started();
    let running: Vec<_> = projector
        .background_jobs()
        .filter(|job| !job.is_finished())
        .map(|job| job.task_id.as_str())
        .collect();
    assert_eq!(running, Vec::<&str>::new());
}

#[test]
fn a_reused_task_id_is_a_new_running_task() {
    let mut projector = Projector::new();
    projector.apply(&ExternalAgentEvent::TaskStarted(TaskStarted {
        task_id: "t1".into(),
        tool_use_id: Some("toolu_old".into()),
        description: Some("old".into()),
        is_backgrounded: true,
    }));
    projector.apply(&finished("t1", "completed"));
    projector.apply(&ExternalAgentEvent::TaskStarted(TaskStarted {
        task_id: "t1".into(),
        tool_use_id: Some("toolu_new".into()),
        description: Some("new".into()),
        is_backgrounded: false,
    }));
    let job = projector
        .background_jobs()
        .find(|job| job.task_id == "t1")
        .expect("t1 is tracked");
    assert!(!job.is_finished(), "the new task runs");
    assert_eq!(job.status, None);
    assert_eq!(job.description.as_deref(), Some("new"));
    assert_eq!(job.tool_use_id.as_deref(), Some("toolu_new"));
    assert!(!job.is_backgrounded);

    // A full list evicts finished jobs first: the reused, running t1 stays.
    for index in 0..BACKGROUND_JOB_CAPACITY + 3 {
        let id = format!("done{index:02}");
        projector.apply(&started(&id));
        projector.apply(&finished(&id, "completed"));
    }
    assert!(
        projector.background_jobs().any(|job| job.task_id == "t1"),
        "the running reused task is not evicted as finished"
    );
}
