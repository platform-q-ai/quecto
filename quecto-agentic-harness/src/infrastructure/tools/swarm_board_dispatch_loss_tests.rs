use serde_json::json;

use super::tests::{captured, running};
use super::{TELEMETRY_TARGET, call};

/// Each loss call (#2277) leaves one INFO record (each is a harness
/// mutation) with its decision and no argument text: a secret-shaped
/// member or exit kind never reaches the log.
#[test]
fn loss_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
    let log = captured(10, |handles| {
        running(handles);
        call(handles, "parent", "_admit", json!(["worker", "r"])).unwrap();
        call(
            handles,
            "parent",
            "_activate",
            json!(["worker", "r", 7, "s", null]),
        )
        .unwrap();
        let quarantined = call(handles, "parent", "_quarantine", json!([secret])).unwrap();
        assert_eq!(quarantined, json!(null));
        call(handles, "parent", "_quarantine", json!(["worker"])).unwrap();
        call(
            handles,
            "parent",
            "_confirmed_dead",
            json!(["worker", secret]),
        )
        .unwrap_err();
        call(handles, "parent", "_confirmed_dead", json!(["worker"])).unwrap();
        call(
            handles,
            "parent",
            "_confirmed_dead",
            json!({"member": "worker"}),
        )
        .unwrap();
        let lost = call(handles, "parent", "_lose_coordinator", json!([])).unwrap();
        assert_eq!(lost["lost"], json!(true));
        call(handles, "parent", "_lose_coordinator", json!([])).unwrap();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(3)
        .collect();
    let expected = [
        ("_quarantine", "ok", "already_lost"),
        ("_quarantine", "ok", "grace_pending"),
        ("_confirmed_dead", "refused", "none"),
        ("_confirmed_dead", "ok", "confirmed"),
        ("_confirmed_dead", "ok", "already_dead"),
        ("_lose_coordinator", "ok", "lost"),
        ("_lose_coordinator", "ok", "not_lost"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            " INFO ".to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            format!("decision=\"{decision}\""),
            "member=\"parent\"".to_owned(),
            "duration_us=".to_owned(),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
}

/// A coordinator's confirmed death ends the run by loss, recorded as its
/// own decision; `_lose_coordinator` answers the run's columns in
/// Python's order.
#[test]
fn loss_answers_render_pythons_shape() {
    let log = captured(4, |handles| {
        running(handles);
        call(handles, "parent", "_admit", json!(["worker", "r"])).unwrap();
        call(handles, "worker", "_confirmed_dead", json!(["parent"])).unwrap();
        let receipt = call(handles, "worker", "_lose_coordinator", json!([])).unwrap();
        assert_eq!(
            receipt.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["id", "status", "outcome", "deadline", "coordinator", "lost"]
        );
        assert_eq!(
            (&receipt["status"], &receipt["outcome"], &receipt["lost"]),
            (&json!("paused"), &json!("failed"), &json!(false))
        );
    });
    assert!(
        log.lines()
            .any(|line| line.contains("op=\"_confirmed_dead\"")
                && line.contains("decision=\"coordinator_confirmed\"")),
        "{log}"
    );
}
