//! Differential scenarios for hand edits of the `run` row, member rows and
//! the control records (#2270, #2273, #2274; epic #2265 P3): what a file
//! edited outside the board leaves, read back through the served methods. Each must match;
//! where the Rust board deliberately differs, the divergence is named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here (listed
//! in its `EXTERNAL_PINS`).
use serde_json::json;

use crate::swarm_board_diff_loss::{GRACE, quarantine};
use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_messages::joined;
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    golden_answer, run_golden, run_rust, sql, step, step_text,
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
        run_golden(&[
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
    run_golden(&[
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
    run_golden(&[
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
        run_golden(&[
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
        run_golden(&[
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
/// (a REAL or a negative sum) are refused so only where the budget decides
/// under a non-null token limit: a paused run's resume check, and a
/// running run's `_record_request` and `_request_admission` (a receipt and
/// the report carry them, as `uncounted_usage_totals_pass_through_elsewhere`
/// pins). Python raises a `TypeError` (or, for a list payload, answers a
/// budget of the totals alone; for a boolean start or uncounted totals,
/// answers as it computes). An integer
/// `started` is read as its float, so an extension from it records a
/// float deadline where Python's records the integer (pinned by
/// `extend_run_deadline_tests`). An event detail that is not an object,
/// or whose `member` is not text (a list or an object Python cannot hash),
/// names no member in the loss scan (a resume's blockers, or #2277's
/// `_quarantine`), where Python raises an `AttributeError` or a
/// `TypeError`. And (#2274) a budget without
/// `token_limit` or `strict_unknown` meets `usage_budget`'s idempotence
/// check, where Python raises a `KeyError`; a stored request whose actor is
/// a BLOB, or whose payload is `'x'` or NULL, meets its redelivery as a
/// store failure, where Python refuses the BLOB as another actor's or
/// raises a `JSONDecodeError` or a `TypeError`. And (#2340) a ledger row
/// whose payload is not JSON, or whose actor is not UTF-8, is not read by
/// a receipt or the budget, which answer where Python's `usage_report`
/// raises or refuses.
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
    // The same through `_quarantine`'s loss scan (#2277 final review N1): a
    // `scope_unknown` or `activated` event whose `member` is a list or an
    // object names no member, so the scan answers as without it, where
    // Python raises a `TypeError` ("unhashable type") testing the member
    // against the wanted set. The scan is `worker`'s observation and, after
    // the grace, its recorded loss, then the run's status.
    let scan = |edits: &[String]| {
        let outcome = run_rust(&joined(
            edits
                .iter()
                .map(|edit| sql(edit))
                .chain([
                    quarantine(3.0, "parent", json!("worker")),
                    quarantine(3.0 + GRACE, "parent", json!("worker")),
                    at(4.0 + GRACE, "parent", "_control_status", json!([])),
                ])
                .collect::<Vec<_>>(),
        ));
        // The receipt less its generation, which counts the edit's event.
        let Outcome::Ok(mut receipt) = outcome else {
            panic!("{edits:?}: {outcome:?}");
        };
        assert!(receipt["generation"].is_u64(), "{receipt}");
        receipt["generation"] = json!(null);
        receipt
    };
    let event = |action: &str, member: &str| {
        format!(
            r#"INSERT INTO events(actor,time,action,detail) VALUES('x',1.0,'{action}','{{"member": {member}}}')"#
        )
    };
    let lost = event("scope_unknown", r#""worker""#);
    assert_ne!(
        scan(&[]),
        scan(std::slice::from_ref(&lost)),
        "the scan sees a loss"
    );
    assert_ne!(
        scan(std::slice::from_ref(&lost)),
        scan(&[lost.clone(), event("activated", r#""worker""#)]),
        "the scan sees an activation"
    );
    for member in [r#"["worker"]"#, r#"{"id": "worker"}"#] {
        assert_eq!(
            scan(&[event("scope_unknown", member)]),
            scan(&[]),
            "scope_unknown {member}"
        );
        assert_eq!(
            scan(&[lost.clone(), event("activated", member)]),
            scan(std::slice::from_ref(&lost)),
            "activated {member}"
        );
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
    unread_ledger_rows_and_totals(&refused);
    // #2340: the process keeps the ledger's sums between transactions, so
    // an edit to a row already summed (its counts, or an earlier row
    // deleted) is not seen by the board that summed it, where Python sums
    // on every read; its last row deleted, or a row added after it (by the
    // board or an edit), is seen by both.
    let counted = |request_id: &str| {
        json!([{"request_id": request_id, "instrumented_attempts": 1, "outcome": "succeeded",
            "context_input_tokens": 2, "output_tokens": 1}])
    };
    for (edit, rusts, pythons) in [
        (
            "UPDATE request_usage SET tokens=100 WHERE request_id='r'",
            6,
            103,
        ),
        ("DELETE FROM request_usage WHERE request_id='r'", 6, 3),
        ("DELETE FROM request_usage", 0, 0),
        (
            "INSERT INTO request_usage VALUES('e','parent','{}',4,0,1,NULL,NULL,NULL,NULL)",
            10,
            10,
        ),
    ] {
        let steps = [
            create(5),
            at(1.0, "parent", "_record_request", counted("r")),
            at(1.5, "parent", "_record_request", counted("s")),
            sql(edit),
            at(2.0, "parent", "_control_status", json!([])),
        ];
        for (side, answer, observed) in [
            ("rust", run_rust(&steps), rusts),
            ("golden", golden_answer(&steps), pythons),
        ] {
            let Outcome::Ok(receipt) = answer else {
                panic!("{edit}: {side}: {answer:?}");
            };
            assert_eq!(
                receipt["budget"]["observed_tokens"],
                json!(observed),
                "{edit}: {side}"
            );
        }
    }
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
        run_golden(&[
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

/// #2340: ledger rows only an edit writes that Python's `usage_report`
/// cannot read, and what Python answers a call reading it: it raises a
/// `JSONDecodeError` for the payload, and refuses the actor as the store's
/// failure (the Rust report refuses both).
/// What Python answers a call: built on demand, as `Outcome` owns text.
type PythonAnswer = fn() -> Outcome;

const UNREADABLE_LEDGER_ROWS: [(&str, PythonAnswer); 2] = [
    (
        "INSERT INTO request_usage VALUES('e','parent','x',1,0,1,NULL,NULL,NULL,NULL)",
        || Outcome::Raised("JSONDecodeError: Expecting value: line 1 column 1 (char 0)".to_owned()),
    ),
    (
        "INSERT INTO request_usage VALUES('e',CAST(x'ff' AS TEXT),'{}',1,0,1,NULL,NULL,NULL,NULL)",
        || {
            Outcome::Refused(
                "coordination store unavailable or contended: Could not decode to UTF-8 column 'member' with text '\u{fffd}'"
                    .to_owned(),
            )
        },
    ),
];

/// `outside_edited_control_records`, the ledger's part (#2340): what the
/// receipt, `_record_request` and `_request_admission` read of the usage
/// report is the budget and its two totals (the board's standing), where
/// Python's read the whole report. Each case asserts both boards' answers.
fn unread_ledger_rows_and_totals(refused: &dyn Fn(&str) -> Outcome) {
    let record = json!([{"request_id": "r", "instrumented_attempts": 1, "outcome": "failed"}]);
    let calls = [
        ("_control_status", json!([])),
        ("_record_request", record),
        ("_request_admission", json!([])),
    ];
    // A ledger row whose payload is not JSON, or whose actor is not UTF-8:
    // Python's report fails on it (the payload among the ten latest); the
    // standing does not read it, so the Rust board answers. The report
    // itself fails on both, on either board.
    for (edit, pythons) in UNREADABLE_LEDGER_ROWS {
        for (method, args) in calls.iter().cloned().chain([("usage_report", json!([]))]) {
            let steps = [
                create(5),
                at(0.5, "parent", "usage_report", json!([])),
                sql(edit),
                at(1.0, "parent", method, args),
            ];
            let (rust, golden) = (run_rust(&steps), golden_answer(&steps));
            assert_eq!(golden, pythons(), "{edit}: {method}");
            match method {
                "usage_report" => {
                    assert!(matches!(rust, Outcome::Refused(_)), "{edit}: {rust:?}");
                }
                _ => assert_answered(method, &rust, (1, None), edit),
            }
        }
    }
    // Another of the report's sums overflowing (two rows of i64::MAX
    // attempts), with or without a limit: Python raises SQLite's `integer
    // overflow` as the store's refusal; the standing sums no attempts, so
    // the Rust board answers, and `_record_request` records its row.
    let overflowing = "INSERT INTO request_usage VALUES('e1','parent','{}',0,0,9223372036854775807,NULL,NULL,NULL,NULL); INSERT INTO request_usage VALUES('e2','parent','{}',0,0,9223372036854775807,NULL,NULL,NULL,NULL)";
    for budget in [json!([null]), json!([1_000, false])] {
        for (method, args) in calls.iter().cloned() {
            let steps = [
                create(5),
                at(0.5, "parent", "usage_budget", budget.clone()),
                sql(overflowing),
                at(1.0, "parent", method, args),
            ];
            assert_eq!(
                golden_answer(&steps),
                Outcome::Refused(
                    "coordination store unavailable or contended: integer overflow".to_owned()
                ),
                "{budget}: {method}"
            );
            let unknown = u64::from(method == "_record_request");
            assert_answered(method, &run_rust(&steps), (0, Some(unknown)), "overflow");
        }
    }
    // A REAL token total on a running run under a limit: the budget's
    // decision refuses it naming the record (`_record_request`,
    // `_request_admission`), where Python compares 10.5 with the limit and
    // answers; the receipt alone carries it on both boards.
    let real = "INSERT INTO request_usage(request_id, actor, payload, tokens) VALUES('e','parent','{}',10.5)";
    for (method, args) in calls {
        let steps = [
            create(5),
            at(0.5, "parent", "usage_budget", json!([1_000, false])),
            sql(real),
            at(1.0, "parent", method, args),
        ];
        let (rust, golden) = (run_rust(&steps), golden_answer(&steps));
        assert!(matches!(golden, Outcome::Ok(_)), "{method}: {golden:?}");
        match method {
            "_control_status" => assert_eq!(rust, golden, "{method}"),
            _ => assert_eq!(rust, refused("usage totals record"), "{method}"),
        }
        if let (Outcome::Ok(answer), "_record_request") = (&golden, method) {
            assert_eq!(answer["budget"]["observed_tokens"], json!(10.5));
        }
    }
}

/// The Rust board answered `method`: a receipt whose observed tokens (and
/// unknown requests, when given) are `totals`, or the admission read of
/// the running run.
fn assert_answered(method: &str, rust: &Outcome, totals: (u64, Option<u64>), case: &str) {
    let Outcome::Ok(answer) = rust else {
        panic!("{case}: {method}: {rust:?}");
    };
    match method {
        "_request_admission" => assert_eq!(answer["status"], json!("running"), "{case}"),
        _ => {
            assert_eq!(
                answer["budget"]["observed_tokens"],
                json!(totals.0),
                "{case}: {method}"
            );
            if let Some(unknown) = totals.1 {
                assert_eq!(
                    answer["budget"]["unknown_usage_requests"],
                    json!(unknown),
                    "{case}: {method}"
                );
            }
        }
    }
}
