use serde_json::json;

use super::{criteria, definition, edited_criteria, evidence_rows, task_record};
use crate::application::swarm::board_test_support::stored_task;
use crate::application::swarm::dto::EvidenceEntry;
use crate::domain::swarm::{CriterionKind, TaskState};

/// The board's criteria parse by id and kind; a missing description reads
/// empty; anything else is refused naming the record.
#[test]
fn criteria_parse_as_the_board_writes_them() {
    let parsed = criteria(&json!([
        {"id": "t", "kind": "command", "description": "d"},
        {"id": "r", "kind": "review"}
    ]))
    .unwrap();
    let read: Vec<_> = parsed
        .iter()
        .map(|c| (c.id.as_str(), c.kind, c.description.as_str()))
        .collect();
    assert_eq!(
        read,
        [
            ("t", CriterionKind::Command, "d"),
            ("r", CriterionKind::Review, "")
        ]
    );
    for edited in [
        json!({"id": "t"}),
        json!(["t"]),
        json!([{"id": 1, "kind": "command"}]),
        json!([{"id": "t", "kind": "manual"}]),
        json!([{"kind": "command"}]),
    ] {
        assert_eq!(
            criteria(&edited).unwrap_err(),
            edited_criteria(),
            "{edited}"
        );
    }
}

/// Only rows with text criterion, revision and a known kind can satisfy a
/// criterion; `accepted` is Python's truthiness of what is stored.
#[test]
fn evidence_rows_keep_only_rows_that_can_satisfy_a_criterion() {
    let row = |revision, kind, accepted| EvidenceEntry {
        criterion: json!("t"),
        artifact: json!("a"),
        revision,
        kind,
        accepted,
    };
    let rows = evidence_rows(&[
        row(json!("R1"), json!("command"), json!(1)),
        row(json!(7), json!("command"), json!(1)),
        row(json!("R1"), json!("other"), json!(1)),
        row(json!("R1"), json!("review"), json!("yes")),
        row(json!("R1"), json!("review"), json!(0)),
    ]);
    let read: Vec<_> = rows.iter().map(|row| (row.kind, row.accepted)).collect();
    assert_eq!(
        read,
        [
            (CriterionKind::Command, true),
            (CriterionKind::Review, true),
            (CriterionKind::Review, false)
        ]
    );
}

/// The first entry whose id equals the criterion by Python's `==`; an
/// entry read before the match that has no id is refused.
#[test]
fn a_definition_is_the_first_entry_with_an_equal_id() {
    let criteria = json!([
        {"id": 1, "kind": "a"},
        {"id": "t", "kind": "b"},
        {"id": "t", "kind": "c"},
        {"kind": "no id"}
    ]);
    let kind = |criterion| {
        definition(&criteria, &criterion)
            .unwrap()
            .map(|fields| fields["kind"].clone())
    };
    assert_eq!(kind(json!("t")), Some(json!("b")));
    assert_eq!(kind(json!(true)), Some(json!("a")), "True == 1");
    assert_eq!(
        definition(&criteria, &json!("x")).unwrap_err(),
        edited_criteria()
    );
    assert_eq!(
        definition(&json!({}), &json!("t")).unwrap_err(),
        edited_criteria()
    );
}

/// A NULL status reads as no known status, never `completed`.
#[test]
fn a_task_record_reads_the_stored_status_and_evidence() {
    let mut task = stored_task(3, "completed", json!([]), Some("w"));
    task.set("evidence", json!([{"revision": "R1"}]));
    let record = task_record(&task);
    assert_eq!(
        (record.id, record.status.as_str(), record.owner.as_deref()),
        (3, "completed", Some("w"))
    );
    assert_eq!(record.evidence, json!([{"revision": "R1"}]));
    task.set("status", json!(null));
    assert_eq!(task_record(&task).status, TaskState::new(""));
}
