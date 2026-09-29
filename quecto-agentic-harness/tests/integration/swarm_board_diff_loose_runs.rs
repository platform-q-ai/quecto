//! Differential scenarios for hand edits of the `run` row and the control
//! records (#2270, #2273, #2274; epic #2265 P3): what a file edited outside the
//! board leaves, read back through the served methods. Each must match;
//! where the Rust board deliberately differs, the divergence is named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here (listed
//! in its `EXTERNAL_PINS`).
use serde_json::json;

use crate::swarm_board_diff_loose::create_text;
use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    run_both, run_rust, sql, step, step_text,
};

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
/// `AttributeError` or a `TypeError`.
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
