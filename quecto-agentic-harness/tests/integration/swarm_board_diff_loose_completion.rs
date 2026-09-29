//! Differential scenarios for hand edits of the run contract and the
//! evidence rows (#2273; epic #2265 P3): what a file edited outside the
//! board leaves, read back through `complete`, `evidence`,
//! `revalidate_task` and `amend`. Each must match; where the Rust board
//! deliberately differs, the divergence is named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here (listed
//! in its `EXTERNAL_PINS`).
use serde_json::json;

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_both, run_rust, sql};

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
