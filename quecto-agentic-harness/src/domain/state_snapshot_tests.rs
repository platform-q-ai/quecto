//! #2210 review: the slim projection's forward-compatible reader, and the
//! unchanged marker that carries a model turn in flight.
use super::reader::{self, Shape};
use super::*;
use serde_json::json;

/// A projection with every member, at every boundary, present.
fn full_json() -> serde_json::Value {
    json!({
        "state": "thinking", "effort": "low", "effortLevels": ["low"],
        "model": "m", "sessionKey": "k",
        "progress": {"state": "active", "reason": "busy"},
        "generation": 3,
        "workflow": {"activeTemplate": {"id": "bugfix"},
                     "currentStep": {"index": 1, "key": "red", "label": "RED",
                                     "phase": "RED", "done": false}},
        "controlReceipts": [{"id": "c", "command": "pause", "status": "queued"}],
        "automaticTurnsSuspended": true,
        "repeatedFailureNotifications": 1,
        "admission": {"waiting": 1, "admitted": 0, "longestWaitSeconds": 4,
                      "groups": [{"group": "g", "cooldown": {"state": "until", "remainingSeconds": 3},
                                  "lastRefusal": "busy"}],
                      "counters": {"completed": 0, "refused": 0, "cancelled": 0, "abandoned": 0},
                      "hidden": 0, "revision": 1, "directory": "/d", "epoch": 1,
                      "connected": true, "authorityStatus": "connected"},
        "admissionWarnings": [{"slot": "s", "code": "c", "message": "m"}],
        "modelTurn": {"elapsedMs": 1, "outputCapBytes": 8,
                      "attempt": {"number": 1, "elapsedMs": 1, "events": 2, "outputBytes": 3,
                                  "sinceLastEventMs": 4, "firstTokenMs": 5}}
    })
}

fn full() -> StateSnapshot {
    serde_json::from_value(full_json()).unwrap()
}

fn members(value: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    names.sort();
    names
}

fn listed(shape: &Shape) -> Vec<String> {
    let mut names: Vec<_> = shape.members.iter().map(|m| m.to_string()).collect();
    names.sort();
    names
}

/// Every boundary's member list is the one the closed types write.
#[test]
fn the_member_lists_are_the_projections_own() {
    let written = serde_json::to_value(full()).unwrap();
    assert_eq!(written, full_json(), "the fixture writes every member");
    for (pointer, shape) in [
        ("", &reader::STATE),
        ("/progress", &reader::PROGRESS),
        ("/workflow", &reader::WORKFLOW),
        ("/workflow/activeTemplate", &reader::TEMPLATE),
        ("/workflow/currentStep", &reader::STEP),
        ("/controlReceipts/0", &reader::RECEIPT),
        ("/admission", &reader::ADMISSION),
        ("/admission/groups/0", &reader::GROUP),
        ("/admission/groups/0/cooldown", &reader::COOLDOWN),
        ("/admission/counters", &reader::COUNTERS),
        ("/admissionWarnings/0", &reader::WARNING),
        ("/modelTurn", &reader::MODEL_TURN),
        ("/modelTurn/attempt", &reader::ATTEMPT),
    ] {
        assert_eq!(
            members(written.pointer(pointer).unwrap()),
            listed(shape),
            "{pointer}"
        );
    }
    let marker = serde_json::to_value(UnchangedSnapshot::at(3, &full())).unwrap();
    assert_eq!(members(&marker), listed(&reader::UNCHANGED));
}

/// The live members the unchanged marker carries are the ones `since`
/// does not hide.
#[test]
fn the_marker_carries_exactly_the_since_bypassing_members() {
    let marker = serde_json::to_value(UnchangedSnapshot::at(3, &full())).unwrap();
    let mut live: Vec<_> = members(&marker)
        .into_iter()
        .filter(|name| !["unchanged", "generation"].contains(&name.as_str()))
        .collect();
    live.sort();
    let mut bypassing: Vec<_> = SINCE_BYPASSING_MEMBERS
        .iter()
        .map(|m| m.to_string())
        .collect();
    bypassing.sort();
    assert_eq!(live, bypassing);
    let mut idle = full();
    idle.model_turn = None;
    assert_eq!(
        serde_json::to_value(UnchangedSnapshot::at(9, &idle)).unwrap(),
        json!({"unchanged": true, "generation": 9})
    );
}

/// A newer writer's added member is dropped at every boundary: the read
/// keeps the known members, and re-serializes without the added one.
#[test]
fn a_newer_writers_added_member_is_dropped_at_every_boundary() {
    for pointer in [
        "",
        "/progress",
        "/workflow",
        "/workflow/currentStep",
        "/controlReceipts/0",
        "/admission",
        "/admission/groups/0",
        "/admission/groups/0/cooldown",
        "/admission/counters",
        "/admissionWarnings/0",
        "/modelTurn",
        "/modelTurn/attempt",
    ] {
        let mut data = full_json();
        data.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("someFutureField".into(), json!({"any": ["shape"]}));
        let read =
            StateSnapshot::read_forward_compatible(&data).unwrap_or_else(|| panic!("{pointer}"));
        assert_eq!(
            serde_json::to_value(read).unwrap(),
            full_json(),
            "{pointer}"
        );
    }
}

/// Known members keep their declared types and required members.
#[test]
fn known_members_are_still_read_strictly() {
    for (pointer, value) in [
        ("/generation", json!("three")),
        ("/progress/state", json!(1)),
        ("/modelTurn/elapsedMs", json!("soon")),
        (
            "/modelTurn/attempt/sinceLastEventMs",
            serde_json::Value::Null,
        ),
        ("/modelTurn/attempt", json!([])),
        ("/admission/groups", json!({})),
        ("/admission/counters/completed", serde_json::Value::Null),
        ("/controlReceipts/0/status", json!("invented")),
    ] {
        let mut data = full_json();
        *data.pointer_mut(pointer).unwrap() = value;
        assert!(
            StateSnapshot::read_forward_compatible(&data).is_none(),
            "{pointer}"
        );
    }
    for pointer in [
        "/modelTurn/attempt/number",
        "/modelTurn/elapsedMs",
        "/progress/reason",
    ] {
        let mut data = full_json();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        data.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(
            StateSnapshot::read_forward_compatible(&data).is_none(),
            "missing {pointer}"
        );
    }
}

#[test]
fn a_member_that_is_no_member_name_is_refused_at_every_boundary() {
    for pointer in [
        "",
        "/modelTurn",
        "/modelTurn/attempt",
        "/admission/groups/0",
    ] {
        for name in ["Upper", "snake_case", "", "1st", "with-dash"] {
            let mut data = full_json();
            data.pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(name.into(), json!(true));
            assert!(
                StateSnapshot::read_forward_compatible(&data).is_none(),
                "{pointer} {name:?}"
            );
        }
    }
}

#[test]
fn a_shape_without_the_required_members_is_refused() {
    // The full session state carries no `state` or `progress`.
    let session = json!({"model": "m", "generation": 7, "isStreaming": true,
                         "sessionKey": "k", "messageCount": 0, "effort": null});
    assert!(StateSnapshot::read_forward_compatible(&session).is_none());
    assert!(StateSnapshot::read_forward_compatible(&json!([])).is_none());
    let marker = json!({"unchanged": true, "generation": 7});
    assert!(StateSnapshot::read_forward_compatible(&marker).is_none());
}

#[test]
fn the_unchanged_marker_is_read_with_its_model_turn() {
    let turn = full_json()["modelTurn"].clone();
    let marker = UnchangedSnapshot::read(&json!({
        "unchanged": true, "generation": 7, "modelTurn": turn
    }))
    .unwrap();
    assert_eq!(marker.generation, 7);
    assert_eq!(marker.model_turn, full().model_turn);
    assert_eq!(
        UnchangedSnapshot::read(&json!({"unchanged": true, "generation": 7})),
        Some(UnchangedSnapshot {
            unchanged: true,
            generation: 7,
            model_turn: None
        })
    );
    // A newer writer's added member is dropped, as in the projection.
    let mut future = json!({"unchanged": true, "generation": 7, "modelTurn": turn});
    future["modelTurn"]["attempt"]["someFutureField"] = json!(1);
    future["someFutureMarker"] = json!(1);
    assert_eq!(UnchangedSnapshot::read(&future), Some(marker.clone()));
    assert_eq!(marker.with_generation(9).generation, 9);
}

#[test]
fn anything_but_the_unchanged_marker_is_refused_as_one() {
    for data in [
        json!({"unchanged": false, "generation": 7}),
        json!({"unchanged": true}),
        json!({"unchanged": true, "generation": 7, "state": "idle"}),
        json!({"unchanged": true, "generation": 7, "modelTurn": {"elapsedMs": "x"}}),
        json!({"unchanged": true, "generation": 7, "Bad-Name": 1}),
        json!([]),
        full_json(),
    ] {
        assert!(UnchangedSnapshot::read(&data).is_none(), "{data}");
    }
}
