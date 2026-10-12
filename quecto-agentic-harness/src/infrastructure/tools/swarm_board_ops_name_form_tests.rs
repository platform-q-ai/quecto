//! #2478: which names a call used, where the swarm's tasks were renamed
//! task items, is recorded as `name_form` on the op's `swarm_op` record and
//! its `swarm structured op` tracing record: `legacy` when the call used
//! any old name (`task`, `tasks`, `task_create`, `verify_task`,
//! `revalidate_task`, `task_id`), `current` when it used task-item names
//! only, and nothing for a call that used neither (an op not renamed,
//! without a task id).
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::super::{execute, refused};
use super::logged;
use crate::domain::swarm::{BoardOpObservation, NameForm};

/// The `name_form` of the record of `op` in `records`, the one record of
/// it; every other record (the harness's own reads) carries none.
fn form_of(records: &[BoardOpObservation], op: &str) -> Option<NameForm> {
    let mine: Vec<&BoardOpObservation> = records.iter().filter(|record| record.op == op).collect();
    assert_eq!(mine.len(), 1, "{op}: {records:?}");
    for other in records.iter().filter(|record| record.op != op) {
        assert_eq!(other.name_form, None, "{other:?}");
    }
    mine[0].name_form
}

#[tokio::test]
async fn a_record_says_which_names_the_call_used() {
    use NameForm::{Current, Legacy};
    let (_directory, context, log) = logged();
    for request in ["r1", "r2", "r3"] {
        let created = execute(
            &context,
            &json!({"op": "task_create", "request": request, "title": "t",
                "acceptance": ["pass"]})
            .to_string(),
        )
        .await;
        assert!(!created.is_error, "{}", created.content);
    }
    log.take();
    let cases = [
        (r#"{"op":"task","task_id":1}"#, "task", Some(Legacy)),
        (
            r#"{"op":"task_item","task_item_id":1}"#,
            "task",
            Some(Current),
        ),
        (r#"{"op":"task_item","task_id":1}"#, "task", Some(Legacy)),
        (r#"{"op":"task","task_item_id":1}"#, "task", Some(Legacy)),
        (
            r#"{"op":"task","task_id":1,"task_item_id":1}"#,
            "task",
            Some(Legacy),
        ),
        (r#"{"op":"tasks"}"#, "tasks", Some(Legacy)),
        (r#"{"op":"task_items"}"#, "tasks", Some(Current)),
        (r#"{"op":"claim","task_id":1}"#, "claim", Some(Legacy)),
        (r#"{"op":"claim","task_item_id":2}"#, "claim", Some(Current)),
        (
            r#"{"op":"task_item_create","request":"r4","title":"t","acceptance":["pass"]}"#,
            "task_create",
            Some(Current),
        ),
        (r#"{"op":"inbox"}"#, "inbox", None),
        (
            r#"{"op":"file_owners","offset":0,"limit":5}"#,
            "file_owners",
            None,
        ),
    ];
    for (text, op, expected) in cases {
        let result = execute(&context, text).await;
        assert!(!result.is_error, "{text}: {}", result.content);
        assert_eq!(form_of(&log.take(), op), expected, "{text}");
    }
    // Refused calls say it too: before the board (a value it cannot hold),
    // by the binding (two values for one argument, an argument the op does
    // not take) and by the board (a task it does not hold).
    let refusals = [
        (
            r#"{"op":"task_item","task_item_id":1e400}"#,
            "task",
            Current,
        ),
        (r#"{"op":"task","task_id":1e400}"#, "task", Legacy),
        // Arguments the board cannot hold still say which names they used.
        (r#"{"op":"claim","task_item_id":1e400}"#, "claim", Current),
        (r#"{"op":"claim","task_id":1e400}"#, "claim", Legacy),
        (
            r#"{"op":"claim","task_id":1,"task_item_id":1e400}"#,
            "claim",
            Legacy,
        ),
        (
            r#"{"op":"claim","task_id":1,"task_item_id":2}"#,
            "claim",
            Legacy,
        ),
        (r#"{"op":"inbox","task_item_id":1}"#, "inbox", Current),
        (r#"{"op":"task_item","task_item_id":99}"#, "task", Current),
    ];
    for (text, op, expected) in refusals {
        refused(&context, text).await;
        assert_eq!(form_of(&log.take(), op), Some(expected), "{text}");
    }
}

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<String>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(bytes));
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedLog {
    type Writer = Self;
    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

/// The `swarm structured op` lines of three calls (current, legacy,
/// neither) under a `fmt` subscriber.
async fn traced_ops() -> Vec<String> {
    let (_directory, context, _log) = logged();
    let captured = CapturedLog::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    for text in [
        r#"{"op":"task_items"}"#,
        r#"{"op":"tasks"}"#,
        r#"{"op":"inbox"}"#,
    ] {
        let result = execute(&context, text).await;
        assert!(!result.is_error, "{text}: {}", result.content);
    }
    drop(guard);
    let traced = captured.0.lock().unwrap().clone();
    traced
        .lines()
        .filter(|line| line.contains("swarm structured op"))
        .map(str::to_owned)
        .collect()
}

/// The structured op's `tracing` record carries the form too (left out
/// for a call that used no renamed name).
///
/// `tracing`'s global level hint is rebuilt without a lock whenever any
/// test thread creates a subscriber (see `swarm_board_ops_input_tests`):
/// a capture missing records reruns, at most five times.
#[tokio::test]
async fn the_tracing_record_says_which_names_the_call_used() {
    let mut lines = traced_ops().await;
    for _ in 0..4 {
        if lines.len() >= 3 {
            break;
        }
        lines = traced_ops().await;
    }
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].contains("name_form=\"current\""), "{}", lines[0]);
    assert!(lines[1].contains("name_form=\"legacy\""), "{}", lines[1]);
    assert!(!lines[2].contains("name_form"), "{}", lines[2]);
}
