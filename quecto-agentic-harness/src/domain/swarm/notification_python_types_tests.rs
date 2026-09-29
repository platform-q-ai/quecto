//! How the wake policy reads loosely typed event details as Python does
//! (#2267 review): ids hashed as Python hashes them, non-string and
//! unhashable values, and the TypeErrors Python's lookups and sort raise.
use serde_json::json;

use super::notification_tests::{NOBODY, event, live, run, state, task, woken};
use super::*;
use crate::domain::swarm::records::TaskState;

fn refusal(
    members: &[MemberRecord],
    events: &[NotificationEvent],
    state: &NotificationState,
) -> String {
    notification_targets(&run(), "worker", members, events, state)
        .expect_err("Python raises TypeError")
        .to_string()
}

#[test]
fn a_float_or_boolean_detail_id_finds_the_task_as_python_hashes_it() {
    // `submit`/`block` bind the agent's task id unchecked, SQLite matches
    // 1.0 and true to row 1, and the event keeps the value as sent. Python's
    // `tasks.get(1.0)` and `tasks.get(True)` find task 1.
    let run = run();
    let members = live(&["parent", "worker"]);
    let board = state(
        vec![
            task(0, TaskState::SUBMITTED, &[], Some("worker")),
            task(1, TaskState::SUBMITTED, &[], Some("worker")),
        ],
        &[],
    );
    for id in [
        json!(1.0),
        json!(true),
        json!(false),
        json!(0.0),
        json!(-0.0),
        json!(1),
    ] {
        let events = [event("submitted", json!({ "task": id }))];
        assert_eq!(
            woken(&run, "worker", &members, &events, &board),
            ["parent"],
            "{id}"
        );
    }
    for id in [
        json!(1.5),
        json!(1e300),
        json!(-1e300),
        json!("1"),
        json!(null),
        json!(9.0),
    ] {
        let events = [event("submitted", json!({ "task": id }))];
        assert_eq!(
            woken(&run, "worker", &members, &events, &board),
            NOBODY,
            "{id}"
        );
    }
}

#[test]
fn a_float_or_boolean_message_id_is_unread_as_python_hashes_it() {
    let run = run();
    let members = live(&["parent", "worker", "other"]);
    let unread = state(Vec::new(), &[1]);
    for id in [json!(1.0), json!(true), json!(1)] {
        let events = [event(
            "message_accepted",
            json!({"message": id, "recipient": "other"}),
        )];
        assert_eq!(
            woken(&run, "worker", &members, &events, &unread),
            ["other"],
            "{id}"
        );
    }
    for id in [json!(1.5), json!(false), json!("1")] {
        let events = [event(
            "message_accepted",
            json!({"message": id, "recipient": "other"}),
        )];
        assert_eq!(
            woken(&run, "worker", &members, &events, &unread),
            NOBODY,
            "{id}"
        );
    }
}

#[test]
fn the_recipient_is_read_only_for_an_unread_message() {
    // Python never evaluates `detail['recipient']` once the message is read.
    let run = run();
    let members = live(&["parent", "worker"]);
    let events = [event("message_accepted", json!({"message": 1}))];
    let read = state(Vec::new(), &[]);
    assert_eq!(woken(&run, "worker", &members, &events, &read), NOBODY);
}

#[test]
fn a_non_string_recipient_alone_wakes_nobody() {
    // `send` binds the recipient unchecked: SQLite matches 5 to member '5'
    // and the event keeps the integer. No live id equals it.
    let run = run();
    let members = live(&["parent", "worker", "5"]);
    let unread = state(Vec::new(), &[1]);
    for recipient in [json!(5), json!(true), json!(5.0)] {
        let events = [event(
            "message_accepted",
            json!({"message": 1, "recipient": recipient}),
        )];
        assert_eq!(
            woken(&run, "worker", &members, &events, &unread),
            NOBODY,
            "{recipient}"
        );
    }
    let events = [
        event("message_accepted", json!({"message": 1, "recipient": 5})),
        event("message_accepted", json!({"message": 1, "recipient": 7.5})),
    ];
    assert_eq!(woken(&run, "worker", &members, &events, &unread), NOBODY);
}

#[test]
fn a_non_string_recipient_beside_a_named_target_fails_as_python_sorted_does() {
    // Python's `sorted(targets)` raises TypeError on a set mixing numbers
    // and strings, so `notifications()` fails for the whole batch.
    let run = run();
    let members = live(&["parent", "worker", "5"]);
    let unread = state(Vec::new(), &[1]);
    for (recipient, kind) in [
        (json!(5), "int"),
        (json!(true), "bool"),
        (json!(5.0), "float"),
    ] {
        let events = [
            event(
                "message_accepted",
                json!({"message": 1, "recipient": recipient}),
            ),
            event("evidence", json!({})),
        ];
        let error = notification_targets(&run, "worker", &members, &events, &unread)
            .expect_err("Python's sort raises");
        assert_eq!(
            error.to_string(),
            format!("'<' not supported between instances of '{kind}' and 'str'")
        );
    }
    // A string recipient beside another named target sorts as usual.
    let events = [
        event("message_accepted", json!({"message": 1, "recipient": "5"})),
        event("evidence", json!({})),
    ];
    assert_eq!(
        woken(&run, "worker", &members, &events, &unread),
        ["5", "parent"]
    );
}

#[test]
fn a_none_recipient_sorts_after_the_named_or_numeric_targets_as_python_does() {
    // CPython 3.12+ hashes None to a constant, so `sorted({None, 'parent'})`
    // always compares 'str' < 'NoneType'.
    let members = live(&["parent", "worker"]);
    let unread = state(Vec::new(), &[1, 2]);
    let none = event("message_accepted", json!({"message": 1, "recipient": null}));
    let five = event("message_accepted", json!({"message": 2, "recipient": 5}));
    let named = event("evidence", json!({}));
    let text =
        |left: &str| format!("'<' not supported between instances of '{left}' and 'NoneType'");
    assert_eq!(
        refusal(&members, &[none.clone(), named.clone()], &unread),
        text("str")
    );
    assert_eq!(
        refusal(&members, &[none.clone(), five.clone(), named], &unread),
        text("str")
    );
    assert_eq!(
        refusal(&members, &[none.clone(), five], &unread),
        text("int")
    );
    assert_eq!(woken(&run(), "worker", &members, &[none], &unread), NOBODY);
}

#[test]
fn an_unhashable_detail_value_fails_as_python_3_14_lookups_do() {
    let members = live(&["parent", "worker"]);
    let board = state(
        vec![task(1, TaskState::SUBMITTED, &[], Some("worker"))],
        &[1],
    );
    for (task_id, kind) in [(json!([1]), "list"), (json!({}), "dict")] {
        let events = [event("submitted", json!({ "task": task_id }))];
        assert_eq!(
            refusal(&members, &events, &board),
            format!("cannot use '{kind}' as a dict key (unhashable type: '{kind}')")
        );
    }
    let events = [event(
        "message_accepted",
        json!({"message": [1], "recipient": "parent"}),
    )];
    assert_eq!(
        refusal(&members, &events, &board),
        "cannot use 'list' as a set element (unhashable type: 'list')"
    );
    for (recipient, kind) in [(json!([1]), "list"), (json!({}), "dict")] {
        let events = [event(
            "message_accepted",
            json!({"message": 1, "recipient": recipient}),
        )];
        assert_eq!(
            refusal(&members, &events, &board),
            format!("cannot use '{kind}' as a set element (unhashable type: '{kind}')")
        );
    }
    // The first failing event decides, as Python raises at once.
    let events = [
        event("blocked", json!({"task": [1]})),
        event(
            "message_accepted",
            json!({"message": [1], "recipient": "parent"}),
        ),
    ];
    assert_eq!(
        refusal(&members, &events, &board),
        "cannot use 'list' as a dict key (unhashable type: 'list')"
    );
}

/// A run row edited outside the board to a NULL status (#2270) is not
/// running, as Python's `None != 'running'`: it wakes nobody.
#[test]
fn a_null_run_status_wakes_nobody() {
    let run = RunRecord {
        status: None,
        ..run()
    };
    let members = live(&["parent", "worker"]);
    let events = [event("amended", json!({}))];
    assert_eq!(
        woken(&run, "worker", &members, &events, &state(Vec::new(), &[])),
        NOBODY
    );
}

/// A member row with a NULL status (#2270) is not live, as Python's
/// `m['status'] == 'live'`: it is never woken.
#[test]
fn a_null_member_status_is_never_woken() {
    let mut members = live(&["parent", "worker"]);
    members.push(MemberRecord {
        id: "odd".into(),
        status: None,
        reservation: None,
    });
    let events = [event("amended", json!({}))];
    assert_eq!(
        woken(&run(), "worker", &members, &events, &state(Vec::new(), &[])),
        ["parent"]
    );
}

/// A NULL coordinator (#2270) adds Python's `None` to the targets: alone it
/// wakes nobody; beside a named target, `sorted(targets)` raises.
#[test]
fn a_null_coordinator_wakes_nobody_and_does_not_sort_beside_a_name() {
    let run = RunRecord {
        coordinator: None,
        ..run()
    };
    let members = live(&["parent", "worker"]);
    let evidence = [event("evidence", json!({}))];
    let board = state(Vec::new(), &[1]);
    assert_eq!(woken(&run, "worker", &members, &evidence, &board), NOBODY);
    let named = [
        event("evidence", json!({})),
        event(
            "message_accepted",
            json!({"message": 1, "recipient": "parent"}),
        ),
    ];
    assert_eq!(
        notification_targets(&run, "worker", &members, &named, &board)
            .expect_err("Python raises TypeError")
            .to_string(),
        "'<' not supported between instances of 'str' and 'NoneType'"
    );
}

/// With a NULL coordinator, `free - {None}` and `takers - {None}` remove
/// nobody: every free member takes ready work, ownership events included.
#[test]
fn a_null_coordinator_is_no_exception_among_ready_work_takers() {
    let run = RunRecord {
        coordinator: None,
        ..run()
    };
    let mut members = live(&["parent", "w"]);
    members.push(MemberRecord {
        id: "x".into(),
        status: None,
        reservation: None,
    });
    let board = state(vec![task(1, TaskState::READY, &[], None)], &[]);
    let created = [event("task_created", json!({"task": 1}))];
    assert_eq!(woken(&run, "parent", &members, &created, &board), ["w"]);
    let claimed = [event("claimed", json!({"task": 1}))];
    assert_eq!(
        woken(&run, "x", &members, &claimed, &board),
        ["parent", "w"]
    );
}
