//! #2279 (parent decision on #2278/#2279): a member's argument text is
//! decoded as Python's `json.loads` decodes it and refused, before any
//! board call, where the board's value type cannot hold it exactly; the
//! answer is written as Python's `json.dumps` writes it; and every
//! structured op is recorded, without its argument text.
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::super::{member_arguments, wire_text};
use super::{Recorded, answered, board, execute, refused};
use crate::domain::swarm::{BoardOpOutcome, RefusalKind};

const UNREPRESENTABLE: &str = "arguments: not representable as a serde_json value: ";

/// Python holds each exactly (or reads it at all); the board's value type
/// cannot, so the op is refused with the store's error shape and nothing
/// reaches the board (P3, pinned differentially by
/// `arguments_beyond_a_serde_value`).
#[tokio::test]
async fn big_integers_non_finite_numbers_and_lone_surrogates_are_refused_before_the_board() {
    let (_directory, context) = board();
    for (text, reason) in [
        (
            r#"{"op":"task_create","request":"r","title":"t","acceptance":["a"],"dependencies":[18446744073709551616]}"#,
            "integer 18446744073709551616 is outside i64 and u64",
        ),
        (
            r#"{"op":"claim","task_id":-9223372036854775809}"#,
            "integer -9223372036854775809 is outside i64 and u64",
        ),
        (
            r#"{"op":"claim","task_id":1e400}"#,
            "Infinity has no JSON number form",
        ),
        (
            r#"{"op":"claim","task_id":NaN}"#,
            "NaN has no JSON number form",
        ),
        (
            r#"{"op":"task_create","request":"r","title":"\ud800","acceptance":["a"]}"#,
            r#""\ud800" holds a lone surrogate"#,
        ),
    ] {
        let content = refused(&context, text).await;
        let expected = format!(
            "swarm: {}",
            Value::String(format!("{UNREPRESENTABLE}{reason}"))
        );
        assert_eq!(content, expected, "{text}");
    }
    assert_eq!(
        context.call("tasks", json!([])).unwrap(),
        json!([]),
        "no call reached the board"
    );
}

/// `json.loads` reads `-0` as the integer 0, which the board's
/// `type(x) is int` accepts (a float `-0.0` would be refused).
#[tokio::test]
async fn minus_zero_is_the_integer_zero() {
    let (_directory, context) = board();
    let tasks = answered(&context, json!({"op": "tasks"})).await;
    let result = execute(&context, r#"{"op":"tasks","offset":-0,"limit":5}"#).await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        serde_json::from_str::<Value>(&result.content).unwrap(),
        tasks
    );
}

#[test]
fn member_arguments_are_read_as_json_loads_reads_them() {
    let read =
        member_arguments(r#"{"a": -0, "b": 1E2, "c": 12345678901234567890, "a": 3}"#).unwrap();
    assert_eq!(
        read.to_string(),
        r#"{"a":3,"b":100.0,"c":12345678901234567890}"#
    );
    assert_eq!(
        member_arguments("[-0]").unwrap().to_string(),
        "[0]",
        "-0 is the integer 0"
    );
    assert_eq!(
        member_arguments("[1e400]").unwrap_err().to_string(),
        format!("{UNREPRESENTABLE}Infinity has no JSON number form")
    );
    assert!(member_arguments("{").is_err());
}

#[test]
fn the_wire_text_is_python_json_dumps() {
    let value = json!({"b": 1e16, "a": 1e-7, "c": "\u{e9}t\u{e9} \u{1f600}", "d": [1, 2.5, null],
        "e": {}, "f": true});
    assert_eq!(
        wire_text(&value),
        r#"{"b": 1e+16, "a": 1e-07, "c": "\u00e9t\u00e9 \ud83d\ude00", "d": [1, 2.5, null], "e": {}, "f": true}"#
    );
    assert_eq!(wire_text(&Value::Null), "null");
}

#[tokio::test]
async fn an_answer_reaches_the_member_as_python_writes_it() {
    let (_directory, context) = board();
    context
        .call("task_create", json!(["r1", "\u{e9}t\u{e9}", ["pass"]]))
        .unwrap();
    let result = execute(&context, r#"{"op":"task","task_id":1}"#).await;
    assert!(!result.is_error, "{}", result.content);
    let expected = wire_text(&context.call("task", json!([1])).unwrap());
    assert_eq!(result.content, expected);
    assert!(
        result.content.starts_with(r#"{"id": 1, "#),
        "{}",
        result.content
    );
    assert!(
        result.content.contains(r#""title": "\u00e9t\u00e9""#),
        "{}",
        result.content
    );
    let nothing = execute(
        &context,
        r#"{"op":"dependencies","task_id":1,"dependencies":[]}"#,
    )
    .await;
    assert_eq!(nothing.content, "null", "a method returning None");
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

/// Each structured op leaves its `swarm_op` records in the event log and
/// one `tracing` record on the board's target (op, gate, lifecycle,
/// sizes), a refusal before the board included; none carries argument
/// text.
#[tokio::test]
async fn structured_ops_are_recorded_without_argument_text() {
    let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let (_directory, context) = board();
    let log = Arc::new(Recorded::default());
    assert!(context.board.record_in(log.clone()));
    let captured = CapturedLog::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    answered(
        &context,
        json!({"op": "send", "request": secret, "recipient": "coordinator",
            "body": format!("key {secret}")}),
    )
    .await;
    let text = format!(r#"{{"op":"claim","task_id":1e400,"note":"{secret}"}}"#);
    refused(&context, &text).await;
    context.pause("hold").unwrap();
    refused(&context, r#"{"op":"inbox"}"#).await;
    drop(guard);
    let records = log.take();
    let written = serde_json::to_string(&records).unwrap();
    assert!(!written.contains(secret), "{written}");
    let outcome = |op: &str| {
        records
            .iter()
            .filter(|record| record.op == op)
            .map(|record| record.outcome)
            .collect::<Vec<_>>()
    };
    assert_eq!(outcome("send"), [BoardOpOutcome::Ok]);
    assert_eq!(
        outcome("claim"),
        [BoardOpOutcome::Refused {
            kind: RefusalKind::Invalid,
            committed: false,
        }],
        "refused arguments are recorded as the op's"
    );
    assert_eq!(
        outcome("inbox"),
        [BoardOpOutcome::Refused {
            kind: RefusalKind::NotRunning,
            committed: false,
        }],
        "the running gate's refusal is recorded as the op's"
    );
    let traced = captured.0.lock().unwrap().clone();
    assert!(!traced.contains(secret), "{traced}");
    let ops: Vec<&str> = traced
        .lines()
        .filter(|line| line.contains("swarm structured op"))
        .collect();
    assert_eq!(ops.len(), 3, "{traced}");
    for (line, expected) in ops.iter().zip([
        ["op=\"send\"", "gate=\"open\"", "lifecycle=\"notified\""],
        ["op=\"claim\"", "gate=\"arguments\"", "lifecycle=\"none\""],
        ["op=\"inbox\"", "gate=\"not_running\"", "lifecycle=\"none\""],
    ]) {
        for field in expected {
            assert!(line.contains(field), "{field} in {line}");
        }
        assert!(line.contains("quecto::swarm_board"), "{line}");
    }
}
