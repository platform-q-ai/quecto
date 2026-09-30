//! #2341: a refusal raised while binding a structured op's arguments
//! (kind `calling` or `invalid`) records which arguments were wrong, by
//! the schema's names only: the missing required ones, the unexpected
//! keys counted (named only when some op's schema has the field), the
//! mis-typed ones with the type the schema expects, and the ones whose
//! value the board cannot hold. No member text reaches the record.
use std::sync::Arc;

use serde_json::json;

use super::{Recorded, board, direct, refused};
use crate::domain::swarm::{
    ArgumentFaults, BoardOpObservation, BoardOpOutcome, RefusalKind, UnexpectedArgs, WrongTypeArg,
};
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// A credential-shaped key, and one shaped like a schema name (lowercase,
/// digits and underscores) that no op's schema has.
const SECRET: &str = "sk-ant-api03-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
const SNAKE_SECRET: &str = "ghp_0123456789abcdef0123456789abcdef0123";

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn wrong(arg: &str, expected: &str) -> WrongTypeArg {
    WrongTypeArg {
        arg: arg.to_owned(),
        expected: expected.to_owned(),
    }
}

/// A coordinator's board recording in its event log, holding task 1.
fn logged() -> (tempfile::TempDir, SwarmContext, Arc<Recorded>) {
    let (directory, context) = board();
    direct(&context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
    let log = Arc::new(Recorded::default());
    assert!(context.board.record_in(log.clone()));
    (directory, context, log)
}

/// The one record `op` left for the refused `text`, refused as `kind`.
async fn refusal(
    context: &SwarmContext,
    log: &Recorded,
    op: &str,
    text: &str,
    kind: RefusalKind,
) -> BoardOpObservation {
    refused(context, text).await;
    let records: Vec<BoardOpObservation> = log
        .take()
        .into_iter()
        .filter(|record| record.op == op)
        .collect();
    assert_eq!(records.len(), 1, "{text}: {records:?}");
    let record = records.into_iter().next().unwrap();
    assert_eq!(
        record.outcome,
        BoardOpOutcome::Refused {
            kind,
            committed: false
        },
        "{text}"
    );
    record
}

/// Every required argument left out is named, not just the first the
/// refusal's text names; the real run's `stop` without its `status` is
/// one of them.
#[tokio::test]
async fn a_missing_argument_records_its_schema_name() {
    let (_directory, context, log) = logged();
    for (op, text, missing) in [
        ("release", r#"{"op":"release","task_id":1}"#, &["token"][..]),
        ("stop", r#"{"op":"stop","reason":"done"}"#, &["status"]),
        (
            "block",
            r#"{"op":"block","task_id":1}"#,
            &["token", "reason"],
        ),
        ("claim", r#"{"op":"claim"}"#, &["task_id"]),
    ] {
        let record = refusal(&context, &log, op, text, RefusalKind::Calling).await;
        assert_eq!(
            record.arguments,
            ArgumentFaults {
                missing_args: names(missing),
                ..ArgumentFaults::NONE
            },
            "{text}"
        );
    }
}

/// Unexpected keys are counted; a key is named only when it is some op's
/// schema field (here `title`, `body` and `goal`, fields of other ops).
#[tokio::test]
async fn unexpected_arguments_are_counted_and_only_schema_fields_named() {
    let (_directory, context, log) = logged();
    let text = r#"{"op":"claim","task_id":1,"owner":"me","title":"t","body":"b","goal":"g"}"#;
    let record = refusal(&context, &log, "claim", text, RefusalKind::Calling).await;
    assert_eq!(
        record.arguments,
        ArgumentFaults {
            unexpected_args: Some(UnexpectedArgs {
                count: 4,
                known: names(&["title", "body", "goal"]),
            }),
            ..ArgumentFaults::NONE
        }
    );
    let text = r#"{"op":"release","task_id":1,"tok":"x"}"#;
    let record = refusal(&context, &log, "release", text, RefusalKind::Calling).await;
    assert_eq!(
        record.arguments,
        ArgumentFaults {
            missing_args: names(&["token"]),
            unexpected_args: Some(UnexpectedArgs {
                count: 1,
                known: vec![],
            }),
            ..ArgumentFaults::NONE
        }
    );
}

/// A known field given a value of another JSON type records its schema
/// name and the type the schema expects (the real run's `task_create`
/// refused as `invalid`).
#[tokio::test]
async fn a_mis_typed_known_field_records_its_expected_type() {
    let (_directory, context, log) = logged();
    for (op, text, expected) in [
        (
            "task_create",
            r#"{"op":"task_create","request":"r2","title":"t","acceptance":"tests pass"}"#,
            wrong("acceptance", "array_of_string"),
        ),
        (
            "task_create",
            r#"{"op":"task_create","request":"r3","title":"t","acceptance":[1]}"#,
            wrong("acceptance", "array_of_string"),
        ),
        (
            "stop",
            r#"{"op":"stop","status":5,"reason":"r"}"#,
            wrong("status", "string"),
        ),
    ] {
        let record = refusal(&context, &log, op, text, RefusalKind::Invalid).await;
        assert_eq!(
            record.arguments,
            ArgumentFaults {
                wrong_type_args: vec![expected],
                ..ArgumentFaults::NONE
            },
            "{text}"
        );
    }
}

/// A refusal the binding did not raise records no argument fault: a
/// well-typed value the board refuses (an empty acceptance list), and a
/// refusal of another kind even where a type differs from the schema's
/// (Python binds a task id untyped, so `"99"` is task 99, which is not
/// found).
#[tokio::test]
async fn a_refusal_the_binding_did_not_raise_records_no_fault() {
    let (_directory, context, log) = logged();
    let text = r#"{"op":"task_create","request":"r2","title":"t","acceptance":[]}"#;
    let record = refusal(&context, &log, "task_create", text, RefusalKind::Invalid).await;
    assert_eq!(record.arguments, ArgumentFaults::NONE);
    let text = r#"{"op":"claim","task_id":"99"}"#;
    let record = refusal(&context, &log, "claim", text, RefusalKind::NotFound).await;
    assert_eq!(record.arguments, ArgumentFaults::NONE);
}

/// Member input the board cannot hold (a huge integer, a number that
/// overflows, `NaN`, a lone surrogate) is refused before the board and
/// records the schema field it was under, or `arguments` when no schema
/// field names it; text that is no JSON records `arguments` on the
/// `unknown` op it is refused as.
#[tokio::test]
async fn unreadable_member_input_records_its_field_or_arguments() {
    let (_directory, context, log) = logged();
    let under_secret = format!(r#"{{"op":"claim","task_id":1,"{SECRET}":1e400}}"#);
    for (op, text, unreadable) in [
        ("claim", r#"{"op":"claim","task_id":1e400}"#.to_owned(), "task_id"),
        ("claim", r#"{"op":"claim","task_id":NaN}"#.to_owned(), "task_id"),
        (
            "task_create",
            r#"{"op":"task_create","request":"r","title":"t","acceptance":["a"],"dependencies":[18446744073709551616]}"#.to_owned(),
            "dependencies",
        ),
        (
            "task_create",
            r#"{"op":"task_create","request":"r","title":"\ud800","acceptance":["a"]}"#.to_owned(),
            "title",
        ),
        ("claim", r#"{"op":"claim","task_id":1,"note":-9223372036854775809}"#.to_owned(), "arguments"),
        ("claim", under_secret, "arguments"),
        ("unknown", r#"{"op":"claim","#.to_owned(), "arguments"),
    ] {
        let record = refusal(&context, &log, op, &text, RefusalKind::Invalid).await;
        assert_eq!(
            record.arguments,
            ArgumentFaults {
                unreadable_args: names(&[unreadable]),
                ..ArgumentFaults::NONE
            },
            "{text}"
        );
        let written = serde_json::to_string(&record).unwrap();
        assert!(!written.contains(SECRET), "{written}");
    }
}

/// A secret-shaped unexpected key is counted, never recorded: neither a
/// credential's shape nor one shaped like a schema name.
#[tokio::test]
async fn a_secret_shaped_unexpected_key_never_reaches_the_record() {
    let (_directory, context, log) = logged();
    let text = format!(
        r#"{{"op":"claim","task_id":1,"{SECRET}":"x","{SNAKE_SECRET}":"{SECRET}","title":"t"}}"#
    );
    let record = refusal(&context, &log, "claim", &text, RefusalKind::Calling).await;
    assert_eq!(
        record.arguments.unexpected_args,
        Some(UnexpectedArgs {
            count: 3,
            known: names(&["title"]),
        })
    );
    let written = serde_json::to_string(&record).unwrap();
    for secret in [SECRET, SNAKE_SECRET] {
        assert!(!written.contains(secret), "{written}");
    }
    assert!(record.arguments.names_are_kinds(), "{record:?}");
}

/// The dispatcher's own binding of a positional call records the same:
/// a required parameter past the values given is missing, and each value
/// past the signature is counted, never named.
#[test]
fn a_positional_call_records_its_missing_and_extra_values() {
    let (_directory, context, log) = logged();
    for (args, expected) in [
        (
            json!([1]),
            ArgumentFaults {
                missing_args: names(&["token"]),
                ..ArgumentFaults::NONE
            },
        ),
        (
            json!([1, "token", SECRET]),
            ArgumentFaults {
                unexpected_args: Some(UnexpectedArgs {
                    count: 1,
                    known: vec![],
                }),
                ..ArgumentFaults::NONE
            },
        ),
    ] {
        direct(&context, "release", args.clone()).unwrap_err();
        let records = log.take();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0].arguments, expected, "{args}");
    }
}
