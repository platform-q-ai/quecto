//! The task-column divergences of the pin table in
//! `swarm_board_diff_loose.rs` (#2272; epic #2265 P3), named in its
//! `PERMITTED_DIVERGENCES` and pinned here by the tests of the same name:
//! `outside_edited_task_columns` and `outside_edited_evidence`.
use serde_json::json;

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, run_rust, sql, try_run_golden,
};

const CONTENDED: &str = "coordination store unavailable or contended: ";

/// `outside_edited_task_columns`: each hand edit of task 1 (after
/// `before`), then the `probe` where the boards differ: `python` is in
/// Python's side of the difference, `rust` in the Rust board's answer.
#[test]
fn outside_edited_task_columns() {
    let setup = [
        create(5),
        at(1.0, "parent", "_admit", json!(["w1", "r1"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["w1", "r1", 11, "t", null]),
        ),
        at(3.0, "parent", "_admit", json!(["w2", "r2"])),
        at(
            4.0,
            "parent",
            "_activate",
            json!(["w2", "r2", 12, "t", null]),
        ),
        at(5.0, "w1", "task_create", json!(["a", "one", ["ok"]])),
        at(6.0, "w1", "task_create", json!(["b", "two", ["ok"]])),
    ];
    let pinned = |before: &[Step], edit: &str, probe: Step, python: &str, rust: &str| {
        let mut steps = setup.to_vec();
        steps.extend_from_slice(before);
        let prefix = format!("step {}: {}", steps.len() + 1, probe.method);
        steps.extend([sql(&format!("UPDATE tasks SET {edit} WHERE id=1")), probe]);
        let difference = try_run_golden(&steps, |_, _, _| {}).unwrap_err();
        let python_side = difference.split("\n  rust").next().unwrap_or_default();
        assert!(
            difference.starts_with(&prefix) && python_side.contains(python),
            "{edit}: {difference}"
        );
        let outcome = format!("{:?}", run_rust(&steps));
        assert!(outcome.contains(rust), "{edit}: {outcome}");
    };
    let raw = || at(7.0, "parent", "task_raw", json!([1]));
    let claim = |member| at(8.0, member, "claim", json!([1]));
    let (ready, blocked) = (
        r#""status": String("ready")"#,
        r#""status": String("blocked")"#,
    );
    for (edit, python, rust) in [
        ("acceptance='not json'", "raised JSONDecodeError", CONTENDED),
        ("evidence=NULL", "raised TypeError", CONTENDED),
        ("dependencies='[99]'", "raised TypeError", blocked),
        (r#"dependencies='{"2":1}'"#, blocked, ready),
        (r#"dependencies='"12"'"#, blocked, ready),
        ("dependencies='null'", "raised TypeError", ready),
        ("dependencies='5'", "raised TypeError", ready),
    ] {
        pinned(&[], edit, raw(), python, rust);
    }
    let unknown = r#"golden Refused("unknown task")"#;
    let unmet = r#"Refused("unmet dependencies")"#;
    pinned(
        &[claim("w1")],
        "dependencies='[99]'",
        claim("w2"),
        unknown,
        unmet,
    );
    // Claim checks the completed dependency using Python's full `_task`,
    // including its acceptance JSON; Rust reads only its status.
    pinned(
        &[
            at(7.0, "w1", "dependencies", json!([2, [1]])),
            sql("UPDATE tasks SET status='completed' WHERE id=1"),
        ],
        "acceptance='not json'",
        at(8.0, "w2", "claim", json!([2])),
        "raised JSONDecodeError",
        r#""status": String("claimed")"#,
    );
    let claimed = r#""status": String("claimed")"#;
    pinned(
        &[],
        r#"dependencies='{"2":1}'"#,
        claim("w1"),
        unmet,
        claimed,
    );
    // The cycle check walks task 1's stored `[[2]]` from a new edge to it.
    for (probe, rust) in [
        (
            at(9.0, "w1", "task_create", json!(["c", "three", ["ok"], [1]])),
            blocked,
        ),
        (at(9.0, "w1", "dependencies", json!([2, [1]])), "Ok("),
    ] {
        pinned(
            &[],
            "dependencies='[[2]]'",
            probe,
            "unhashable type: 'list'",
            rust,
        );
    }
}

/// `outside_edited_evidence`: Python raises, or verifies an empty dict or
/// text, where the Rust board refuses.
#[test]
fn outside_edited_evidence() {
    let task = at(1.0, "parent", "task_create", json!(["r", "t", ["ok"]]));
    let mut steps = vec![create(5), task];
    for (evidence, python) in [
        (r#"[{"artifact":"a"}]"#, "Python raised KeyError"),
        (r#"{"revision":"R1"}"#, "Python raised TypeError"),
        ("null", "Python raised TypeError"),
        ("{}", "golden Ok(Null)"),
        (r#""""#, "golden Ok(Null)"),
    ] {
        steps.truncate(2);
        let edit = format!("UPDATE tasks SET status='submitted',token='t',evidence='{evidence}'");
        steps.extend([
            sql(&edit),
            at(2.0, "parent", "verify_task", json!([1, "t", "R1"])),
        ]);
        let difference = try_run_golden(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with("step 3: verify_task") && difference.contains(python),
            "{evidence}: {difference}"
        );
        let outcome = format!("{:?}", run_rust(&steps));
        assert!(outcome.contains("not a list of revisioned"), "{outcome}");
    }
}
