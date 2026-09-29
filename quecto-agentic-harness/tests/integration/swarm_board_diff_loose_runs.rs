//! Differential scenarios for hand edits of the `run` row, member rows and
//! the control records (#2270, #2273, #2274; epic #2265 P3): what a file
//! edited outside the board leaves, read back through the served methods. Each must match;
//! where the Rust board deliberately differs, the divergence is named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here (listed
//! in its `EXTERNAL_PINS`).
use serde_json::json;

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    run_both, run_rust, sql, step, step_text,
};

/// `create_run`'s arguments as JSON text, a criterion's extra key `w`
/// written as `extra`.
pub(crate) fn create_text(extra: &str) -> String {
    format!(
        r#"["g", [], [{{"id": "c", "kind": "review", "description": "d", "w": {extra}}}], 5, {}]"#,
        NOW + 3_600.0
    )
}

/// `_bootstrap` asks only whether a run exists (`SELECT 1 FROM run`) and
/// `create` fetches the whole run row but uses only its status and
/// coordinator (#2270 round-3 review N1): columns either leaves unused may
/// hold anything.
#[test]
fn bootstrap_and_create_use_only_the_run_columns_python_uses() {
    for edit in [
        "UPDATE run SET deadline='soon', member_limit='many'",
        "UPDATE run SET deadline=NULL, member_limit=2.5, integrator=x'00'",
    ] {
        run_both(&[
            step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
            sql(edit),
            step("parent", "bootstrap_run", json!([8, "t", null]), NOW + 1.0),
            step_text("parent", "create_run", &create_text("1"), NOW + 2.0),
            step("supervisor", "_status", json!([]), NOW + 3.0),
        ]);
    }
}

/// Rows the board never writes itself, read as Python reads them: a NULL
/// member status, a NULL coordinator, a pid of any storage class, and
/// `_status` reading only the columns Python's `_status` selects.
#[test]
fn loosely_typed_rows_read_as_python_reads_them() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        sql("INSERT INTO members(id,status) VALUES('odd',NULL)"),
        sql("INSERT INTO members(id,status,pid) VALUES('text','live','abc')"),
        sql("INSERT INTO members(id,status,pid) VALUES('real','reserved',2.5)"),
        step("parent", "_snapshot", json!([]), NOW + 1.0),
        step("odd", "_snapshot", json!([]), NOW + 2.0),
        step("supervisor", "_status", json!([]), NOW + 3.0),
        sql("UPDATE members SET status=NULL WHERE id='parent'"),
        step("parent", "_snapshot", json!([]), NOW + 4.0),
        sql("UPDATE run SET coordinator=NULL"),
        step("parent", "_snapshot", json!([]), NOW + 5.0),
        step("supervisor", "_status", json!([]), NOW + 6.0),
        // `create` over the placeholder needs its coordinator.
        step(
            "parent",
            "create_run",
            json!(["g", [], [{"id": "c", "kind": "review", "description": "d"}], 5, NOW + 3_600.0]),
            NOW + 7.0,
        ),
    ]);
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        // Columns `_status` does not select, and a NULL status it does.
        sql("UPDATE run SET member_limit='many', goal=x'00', criteria=NULL, integrator=3"),
        step("supervisor", "_status", json!([]), NOW + 1.0),
        sql("UPDATE run SET status=NULL, outcome=4"),
        step("supervisor", "_status", json!([]), NOW + 2.0),
    ]);
}

/// Text that is not UTF-8 in any column of a row Python fetches is refused
/// with Python's text (#2270 round-4 review L1, L3), its bytes shown with
/// one U+FFFD each: a run's status, coordinator or goal under `create`
/// (which fetches the whole row), `_snapshot` and `_status`, and a
/// member's `started` under `_snapshot`. A status or coordinator that is
/// not text (a BLOB, or a number in a table rebuilt without TEXT affinity)
/// is not the setup placeholder, so `create` refuses to reset it; a number
/// the TEXT affinity stores as its text is not the placeholder either.
#[test]
fn outside_edited_text_reads_as_python_reads_it() {
    let create = |now| step_text("parent", "create_run", &create_text("1"), now);
    for edit in [
        "UPDATE run SET status=CAST(x'ff' AS TEXT)",
        "UPDATE run SET coordinator=CAST(x'706172e282ff656e74' AS TEXT)",
        "UPDATE run SET goal=CAST(x'ff' AS TEXT)",
        "UPDATE members SET started=CAST(x'31ff32' AS TEXT)",
    ] {
        run_both(&[
            step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
            sql(edit),
            create(NOW + 1.0),
            step("parent", "_snapshot", json!([]), NOW + 2.0),
            step("supervisor", "_status", json!([]), NOW + 3.0),
        ]);
    }
    for edit in [
        "UPDATE run SET status=x'7365747570'",
        "UPDATE run SET status=1",
        "UPDATE run SET coordinator=x'706172656e74'",
        "UPDATE run SET coordinator=2.5",
        "DROP TABLE run;
         CREATE TABLE run (id, goal, constraints, criteria, coordinator, integrator, member_limit, deadline, status);
         INSERT INTO run(id,status,coordinator) VALUES('r',1,2.5)",
    ] {
        run_both(&[
            step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
            sql(edit),
            create(NOW + 1.0),
        ]);
    }
}

/// `outside_edited_control_records` (#2273), records only a file edited
/// outside the board holds: a pause record whose `started` is not a
/// number (a boolean included, which Python counts as 0 or 1), or a
/// budget payload that is not an object (or whose token limit is not a
/// count), is refused naming the record; usage totals that are not counts
/// (a REAL or a negative sum) are refused so only when a paused run's
/// budget, with a non-null token limit, is checked for a resume (elsewhere
/// they pass through, as `uncounted_usage_totals_pass_through_elsewhere`
/// pins). Python raises a `TypeError` (or, for a list payload, answers a
/// budget of the totals alone; for a boolean start or uncounted totals,
/// answers as it computes). An integer
/// `started` is read as its float, so an extension from it records a
/// float deadline where Python's records the integer (pinned by
/// `extend_run_deadline_tests`). An event detail that is not an object,
/// or whose `member` is not text (a list or an object Python cannot hash),
/// names no member in the loss scan, where Python raises an
/// `AttributeError` or a `TypeError`. And (#2274) a budget without
/// `token_limit` or `strict_unknown` meets `usage_budget`'s idempotence
/// check, where Python raises a `KeyError`; a stored request whose actor is
/// a BLOB, or whose payload is `'x'` or NULL, meets its redelivery as a
/// store failure, where Python refuses the BLOB as another actor's or
/// raises a `JSONDecodeError` or a `TypeError`.
#[test]
fn outside_edited_control_records() {
    let paused = |edit: &str| {
        run_rust(&[
            create(5),
            at(1.0, "parent", "pause", json!(["hold"])),
            at(2.0, "parent", "_control_status", json!([])),
            sql(edit),
            at(3.0, "parent", "_control_status", json!([])),
        ])
    };
    let refused = |record: &str| {
        Outcome::Refused(format!(
            "the board's {record} is not as the board writes it"
        ))
    };
    assert_eq!(
        paused(r#"UPDATE events SET detail='{"started": "soon"}' WHERE action='paused'"#),
        refused("pause record")
    );
    assert_eq!(
        paused("INSERT INTO usage_budget VALUES(1, '[]')"),
        refused("usage budget")
    );
    assert_eq!(
        paused(
            r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": "10", "strict_unknown": false}')"#
        ),
        refused("usage budget")
    );
    assert_eq!(
        paused(r#"UPDATE events SET detail='{"started": true}' WHERE action='paused'"#),
        refused("pause record")
    );
    assert_eq!(
        paused(concat!(
            r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": 100, "strict_unknown": false}');"#,
            "INSERT INTO request_usage(request_id, actor, payload, tokens) VALUES('r', 'parent', '{}', 1.5);",
        )),
        refused("usage totals record")
    );
    assert_eq!(
        paused(concat!(
            r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": 100, "strict_unknown": false}');"#,
            "INSERT INTO request_usage(request_id, actor, payload, tokens) VALUES('r', 'parent', '{}', -3);",
        )),
        refused("usage totals record")
    );
    let extended = run_rust(&[
        create(5),
        at(1.0, "parent", "pause", json!(["hold"])),
        sql(&format!(
            r#"UPDATE events SET detail='{{"started": {}}}' WHERE action='paused'"#,
            NOW as i64 + 3_700
        )),
        at(2.0, "parent", "_extend_deadline", json!([60])),
    ]);
    let Outcome::Ok(receipt) = extended else {
        panic!("{extended:?}");
    };
    assert_eq!(receipt["status"], json!("paused"), "{receipt}");
    for detail in [
        r#"'["parent"]'"#,
        r#"'{"member": ["parent"]}'"#,
        r#"'{"member": {"id": "parent"}}'"#,
    ] {
        let unnamed = paused(&format!(
            "INSERT INTO events(actor,time,action,detail) VALUES('x',1.0,'scope_unknown',{detail})"
        ));
        let Outcome::Ok(receipt) = unnamed else {
            panic!("{detail}: {unnamed:?}");
        };
        assert_eq!(receipt["resume_blockers"], json!([]), "{detail}: {receipt}");
    }
    // #2274: a stored request that is not an object meets its redelivery,
    // and a budget without `warned` meets the warning, where Python raises
    // a `TypeError` or a `KeyError`.
    let record = json!([{"request_id": "r", "instrumented_attempts": 1, "outcome": "failed"}]);
    let redelivered = |edit: &str| {
        run_rust(&[
            create(5),
            at(1.0, "parent", "_record_request", record.clone()),
            sql(edit),
            at(2.0, "parent", "_record_request", record.clone()),
        ])
    };
    assert_eq!(
        redelivered("UPDATE request_usage SET payload='[]'"),
        refused("request usage")
    );
    assert_eq!(
        redelivered(r#"UPDATE request_usage SET payload='"r"'"#),
        refused("request usage")
    );
    // A stored actor that is a BLOB, or a payload that is not JSON text
    // (`'x'`, NULL), is a store failure naming the column, where Python
    // refuses the BLOB as another actor's (`bytes != str`) and raises a
    // `JSONDecodeError` or a `TypeError` for the payload.
    for (edit, failure) in [
        (
            "UPDATE request_usage SET actor=x'706172656e74'",
            "Invalid column type Blob at index: 0, name: actor",
        ),
        (
            "UPDATE request_usage SET payload='x'",
            "Conversion error from type Text at index: 1, Expecting value: line 1 column 1 (char 0)",
        ),
        (
            "UPDATE request_usage SET payload=NULL",
            "Invalid column type Null at index: 1, name: payload",
        ),
    ] {
        assert_eq!(
            redelivered(edit),
            Outcome::Refused(format!(
                "coordination store unavailable or contended: {failure}"
            )),
            "{edit}"
        );
    }
    assert_eq!(
        run_rust(&[
            create(5),
            at(0.5, "parent", "usage_report", json!([])),
            sql(
                r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": 1, "strict_unknown": false}')"#
            ),
            at(
                1.0,
                "parent",
                "_record_request",
                json!([{"request_id": "r", "instrumented_attempts": 1, "outcome": "succeeded", "context_input_tokens": 1, "output_tokens": 1}])
            ),
        ]),
        refused("usage budget")
    );
    // A budget without `token_limit`, or without `strict_unknown` where the
    // limit is the one asked for, meets `usage_budget`'s idempotence check,
    // where Python raises a `KeyError`.
    for (edit, limit) in [
        (
            r#"INSERT INTO usage_budget VALUES(1, '{"strict_unknown": true, "warned": false}')"#,
            json!(5),
        ),
        (
            r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": 5, "warned": false}')"#,
            json!(5),
        ),
        (
            r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": null, "warned": false}')"#,
            json!(null),
        ),
    ] {
        assert_eq!(
            run_rust(&[
                create(5),
                at(0.5, "parent", "usage_report", json!([])),
                sql(edit),
                at(1.0, "parent", "usage_budget", json!([limit, true])),
            ]),
            refused("usage budget"),
            "{edit}"
        );
    }
}

/// Usage totals that are not counts (a REAL or a negative sum) are refused
/// only where a paused run's budget with a non-null token limit is checked
/// for a resume (#2318 final review): a running run's receipt and usage
/// report, and a paused run's under a null limit, carry them as Python
/// does.
#[test]
fn uncounted_usage_totals_pass_through_elsewhere() {
    for tokens in ["1.5", "-3"] {
        run_both(&[
            create(5),
            at(1.0, "parent", "usage_report", json!([])),
            sql(&format!(
                "INSERT INTO request_usage(request_id, actor, payload, tokens) VALUES('r', 'parent', '{{}}', {tokens});"
            )),
            at(2.0, "parent", "_control_status", json!([])),
            at(3.0, "parent", "usage_report", json!([])),
            sql(
                r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": null, "strict_unknown": false}')"#,
            ),
            at(4.0, "parent", "pause", json!(["hold"])),
            at(5.0, "parent", "_control_status", json!([])),
        ]);
    }
}
