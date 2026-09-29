//! Differential scenarios for hand edits of the run contract and the
//! evidence rows (#2273; epic #2265 P3): what a file edited outside the
//! board leaves, read back through `complete`, `evidence`,
//! `revalidate_task` and `amend`. Each must match; where the Rust board
//! deliberately differs, the divergence is named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here (listed
//! in its `EXTERNAL_PINS`).
//!
//! `outside_edited_contract`, case by case (each pinned below):
//!
//! - Malformed stored JSON in the criteria is a store refusal in Rust,
//!   where Python's `json.loads` raises in `complete` and `evidence`;
//!   `amend` likewise loads the stored constraints and criteria as JSON,
//!   but validates neither shape.
//! - `complete` validates the criteria's shape in Rust, where Python
//!   checks each criterion's `id` and `kind` only when it reaches that
//!   entry (or refuses missing evidence first).
//! - `evidence` checks a matching criterion's kind in Rust: a missing kind
//!   is refused naming the record, where Python raises, and a kind that is
//!   not text (`{"id": "t", "kind": 1}`) is refused as not matching even
//!   when the caller passes the same value, where Python's `!=` finds them
//!   equal and records the row; a non-text id equal to the requested id
//!   can match.
//! - A task's acceptance and dependencies are not loaded by Python's
//!   `complete` (`Transaction.task` loads only the evidence), but Rust's
//!   `task_row` loads all three JSON columns and refuses malformed
//!   acceptance or dependencies as a store failure. Malformed task
//!   evidence JSON raises in Python and is a store refusal in Rust.
//! - A completed task's hand-edited evidence that is truthy but not a list
//!   of objects each carrying `revision` (a nonempty object, text, a
//!   number or `true` in place of the list, or a list with an entry that
//!   lacks `revision` or is not an object) makes Python's `complete` raise
//!   `TypeError` or `KeyError`, while Rust refuses it as stale task
//!   evidence.
use serde_json::json;

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_both, run_rust, sql};

/// The prefix of a store failure's refusal.
const CONTENDED: &str = "coordination store unavailable or contended: ";

/// Evidence rows the board never writes (another revision, an unknown
/// kind, a text `accepted`, which INTEGER affinity keeps when it is not
/// numeric) are compared as Python compares them: only a row equal to a
/// criterion's id and kind at the revision, with a truthy `accepted`,
/// counts.
#[test]
fn edited_evidence_rows_are_compared_as_python_compares_them() {
    run_both(&[
        create(5),
        sql("INSERT INTO evidence VALUES('t','a','R0','command','parent',1)"),
        at(1.0, "parent", "complete", json!(["R1"])),
        sql("INSERT INTO evidence VALUES('t','a','R1','other','worker',1)"),
        at(2.0, "parent", "complete", json!(["R1"])),
        sql("INSERT INTO evidence VALUES('t','a','R1','command','x','')"),
        at(3.0, "parent", "complete", json!(["R1"])),
        sql("UPDATE evidence SET accepted='yes' WHERE actor='x'"),
        at(4.0, "parent", "complete", json!(["R1"])),
    ]);
}

/// `outside_edited_contract` (#2273): after criterion evidence is accepted,
/// a completed task whose evidence was hand-edited to something truthy
/// that is not a list of revisioned entries makes Python's `complete`
/// raise while Rust refuses stale task evidence. The row is never written
/// by the board itself.
#[test]
fn complete_with_outside_edited_task_evidence() {
    // Independent Python swarm_policy.completion probe: an entry without
    // `revision` raises KeyError('revision'); `[1]` raises TypeError("'int'
    // object is not subscriptable"); an object or text in place of the
    // list iterates its keys or characters and raises TypeError ("string
    // indices must be integers"); a number or `true` raises TypeError
    // ("object is not iterable"). No harness difference hook is needed for
    // this Rust-only pin of the deliberately permitted divergence.
    for evidence in [
        r#"[{"artifact":"a"}]"#,
        "[1]",
        r#"{"revision":"R1"}"#,
        r#""R1""#,
        "1",
        "true",
    ] {
        let steps = [
            create(5),
            sql("INSERT INTO evidence VALUES('t','a','R1','command','parent',1)"),
            at(
                1.0,
                "parent",
                "task_create",
                json!(["task", "title", ["ok"]]),
            ),
            sql(&format!(
                "UPDATE tasks SET status='completed', evidence='{evidence}' WHERE id=1"
            )),
            at(2.0, "parent", "complete", json!(["R1"])),
        ];
        assert_eq!(
            run_rust(&steps),
            Outcome::Refused("task evidence refers to stale revision".to_owned()),
            "{evidence}"
        );
    }
}

/// `outside_edited_contract` (#2273), a contract only a file edited
/// outside the board holds: criteria that are not a list of objects each
/// with a text id and a `command` or `review` kind are refused naming the
/// record by `complete` (and by `evidence` where Python raises), where
/// Python compares what it finds or raises; constraints or criteria that
/// are not JSON text are refused as a store failure by `amend` before its
/// update, where Python raises after it.
#[test]
fn outside_edited_contract() {
    let edited = |edit: &str, method: &str, args: serde_json::Value| {
        run_rust(&[create(5), sql(edit), at(1.0, "parent", method, args)])
    };
    let refused =
        || Outcome::Refused("the board's run criteria is not as the board writes it".to_owned());
    assert_eq!(
        edited(
            r#"UPDATE run SET criteria='[{"id": "t", "kind": "other"}]'"#,
            "complete",
            json!(["R1"])
        ),
        refused()
    );
    assert_eq!(
        edited(
            r#"UPDATE run SET criteria='{"t": 1}'"#,
            "evidence",
            json!(["t", "a", "R1", "command", true])
        ),
        refused()
    );
    let amended = edited(
        "UPDATE run SET constraints='not json'",
        "amend",
        json!(["g", [], [{"id": "t", "kind": "review", "description": "d"}], "r"]),
    );
    assert!(
        matches!(&amended, Outcome::Refused(text) if text.starts_with("coordination store unavailable or contended: ")),
        "{amended:?}"
    );
}

/// A file edit can put malformed JSON into the contract or task columns,
/// or omit a criterion's kind. Pin each read independently: Python's
/// `complete` loads criteria and task evidence only, while Rust additionally
/// loads task acceptance and dependencies; `evidence` looks up the
/// criterion's kind.
#[test]
fn outside_edited_contract_malformed_fields_are_pinned() {
    let cases = [
        (
            "UPDATE run SET criteria='not json'",
            "complete",
            json!(["R1"]),
            CONTENDED,
        ),
        (
            "UPDATE run SET criteria='not json'",
            "evidence",
            json!(["c", "a", "R1", "review", true]),
            CONTENDED,
        ),
        (
            "UPDATE tasks SET acceptance='not json'",
            "complete",
            json!(["R1"]),
            CONTENDED,
        ),
        (
            "UPDATE tasks SET dependencies='not json'",
            "complete",
            json!(["R1"]),
            CONTENDED,
        ),
        (
            r#"UPDATE run SET criteria='[{"id":"c"}]'"#,
            "evidence",
            json!(["c", "a", "R1", "review", true]),
            "the board's run criteria is not as the board writes it",
        ),
    ];
    for (edit, method, args, rust) in cases {
        let steps = [
            create(5),
            at(1.0, "parent", "task_create", json!(["r", "t", ["ok"]])),
            sql(edit),
            at(2.0, "parent", method, args),
        ];
        // Python's independent Transaction.completion_state/task probe:
        // malformed criteria raises JSONDecodeError; missing kind raises
        // KeyError; malformed acceptance/dependencies are not loaded, so
        // the unmet criterion produces the ordinary completion refusal.
        let outcome = run_rust(&steps);
        assert!(format!("{outcome:?}").contains(rust), "{edit}: {outcome:?}");
    }
}

/// `outside_edited_contract` (#2273): a criterion whose stored kind is not
/// text (`1`, only an edit writes it) matches no evidence in Rust, even
/// evidence passed with the same value, where Python's `!=` finds `1 == 1`
/// and records the row.
#[test]
fn a_criterion_kind_that_is_not_text_matches_no_evidence() {
    let steps = [
        create(5),
        sql(r#"UPDATE run SET criteria='[{"id":"t","kind":1}]'"#),
        at(1.0, "parent", "evidence", json!(["t", "a", "R1", 1, true])),
    ];
    // Python's independent Workbench.evidence probe: `definition['kind'] !=
    // kind` is False, so it records ('t','a','R1',1,'parent',1).
    assert_eq!(
        run_rust(&steps),
        Outcome::Refused("evidence must match a configured criterion and kind".to_owned())
    );
}
