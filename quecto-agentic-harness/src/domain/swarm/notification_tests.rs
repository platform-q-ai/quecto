//! Ported from `tests/swarm_policy_test.py::PolicyContract` (notification
//! and wake policy), plus the Rust-only edges of the event detail.
use serde_json::{Value, json};

use super::*;
use crate::domain::swarm::records::{MemberState, RunState};

fn run() -> RunRecord {
    RunRecord {
        status: RunState::RUNNING,
        coordinator: "parent".into(),
        deadline: 100.0,
        member_limit: 2,
        outcome: None,
        outcome_reason: None,
    }
}

fn member(id: &str, status: MemberState) -> MemberRecord {
    MemberRecord {
        id: id.into(),
        status,
        reservation: None,
    }
}

fn live(ids: &[&str]) -> Vec<MemberRecord> {
    ids.iter().map(|id| member(id, MemberState::LIVE)).collect()
}

fn task(id: i64, status: TaskState, dependencies: &[i64], owner: Option<&str>) -> TaskSummary {
    TaskSummary {
        id,
        status,
        dependencies: dependencies.to_vec(),
        owner: owner.map(str::to_owned),
    }
}

fn state(tasks: Vec<TaskSummary>, unread: &[i64]) -> NotificationState {
    NotificationState {
        tasks,
        unread: unread.iter().copied().collect(),
    }
}

fn event(action: &str, detail: Value) -> NotificationEvent {
    NotificationEvent {
        action: action.into(),
        detail,
        actor: None,
    }
}

fn by(action: &str, detail: Value, actor: &str) -> NotificationEvent {
    NotificationEvent {
        actor: Some(actor.into()),
        ..event(action, detail)
    }
}

fn woken(
    run: &RunRecord,
    actor: &str,
    members: &[MemberRecord],
    events: &[NotificationEvent],
    state: &NotificationState,
) -> Vec<String> {
    notification_targets(run, actor, members, events, state)
        .expect("the batch sorts")
        .into_iter()
        .map(|member| member.id)
        .collect()
}

const NOBODY: [&str; 0] = [];

#[test]
fn notification_policy_ignores_bookkeeping_and_terminal_work() {
    let mut run = run();
    let members = live(&["parent", "worker"]);
    let board = state(vec![task(1, TaskState::SUBMITTED, &[], None)], &[1]);
    let mut events = vec![
        event("message_consumed", json!({})),
        event("claimed", json!({})),
        event("files_reserved", json!({})),
    ];
    assert_eq!(woken(&run, "worker", &members, &events, &board), NOBODY);
    events.push(event("submitted", json!({"task": 1})));
    events.push(event(
        "message_accepted",
        json!({"message": 1, "recipient": "parent"}),
    ));
    assert_eq!(woken(&run, "worker", &members, &events, &board), ["parent"]);
    // A message that is no longer unread (superseded or withdrawn, #1837) wakes nobody.
    let retired = state(Vec::new(), &[]);
    let accepted_only = [event(
        "message_accepted",
        json!({"message": 1, "recipient": "parent"}),
    )];
    assert_eq!(
        woken(&run, "worker", &members, &accepted_only, &retired),
        NOBODY
    );
    run.status = RunState::SUCCEEDED;
    assert_eq!(woken(&run, "worker", &members, &events, &board), NOBODY);
}

#[test]
fn ready_work_wakes_only_members_free_to_take_it() {
    // #2127: a parked, busy or stuck member is not woken because other work
    // changed; a free member is. A message addressed to anyone wakes them.
    let run = run();
    let members = live(&["parent", "free", "parked", "busy", "stuck"]);
    let board = state(
        vec![
            task(1, TaskState::READY, &[], None),
            task(2, TaskState::SUBMITTED, &[], Some("parked")),
            task(3, TaskState::CLAIMED, &[], Some("busy")),
            task(4, TaskState::BLOCKED, &[], Some("stuck")),
        ],
        &[9],
    );
    let verified_other = [event("verified", json!({"task": 5}))];
    assert_eq!(
        woken(&run, "parent", &members, &verified_other, &board),
        ["free"]
    );
    let message = [event(
        "message_accepted",
        json!({"message": 9, "recipient": "parked"}),
    )];
    assert_eq!(
        woken(&run, "parent", &members, &message, &board),
        ["parked"]
    );
}

#[test]
fn parked_members_take_new_work_when_no_worker_is_free() {
    let run = run();
    let members = live(&["parent", "parked", "busy"]);
    let board = state(
        vec![
            task(1, TaskState::READY, &[], None),
            task(2, TaskState::SUBMITTED, &[], Some("parked")),
            task(3, TaskState::CLAIMED, &[], Some("busy")),
        ],
        &[],
    );
    let created = [event("task_created", json!({"task": 1}))];
    assert_eq!(
        woken(&run, "parent", &members, &created, &board),
        ["parked"]
    );
}

#[test]
fn no_ready_work_wakes_nobody_and_a_member_holding_two_tasks_stays_busy() {
    let run = run();
    let members = live(&["parent", "free", "double"]);
    let mut tasks = vec![
        task(1, TaskState::COMPLETED, &[], Some("free")),
        task(2, TaskState::SUBMITTED, &[], Some("double")),
        task(3, TaskState::CLAIMED, &[], Some("double")),
    ];
    let verified = [event("verified", json!({"task": 1}))];
    let board = state(tasks.clone(), &[]);
    assert_eq!(woken(&run, "parent", &members, &verified, &board), NOBODY);
    tasks.push(task(4, TaskState::READY, &[], None));
    let board = state(tasks, &[]);
    assert_eq!(woken(&run, "parent", &members, &verified, &board), ["free"]);
}

#[test]
fn a_release_hands_the_work_on_and_the_receiver_agrees() {
    let run = run();
    let members = live(&["parent", "releaser", "parked"]);
    let board = state(
        vec![
            task(1, TaskState::READY, &[], None),
            task(2, TaskState::SUBMITTED, &[], Some("parked")),
        ],
        &[],
    );
    let released = [by("released", json!({"task": 1}), "releaser")];
    assert_eq!(
        woken(&run, "releaser", &members, &released, &board),
        ["parent", "parked"]
    );
    assert_eq!(
        woken(&run, "", &members, &released, &board),
        ["parent", "parked"]
    );
}

#[test]
fn the_fallback_wakes_every_parked_member_but_the_actor() {
    let run = run();
    let members = live(&["parent", "p1", "p2", "p3"]);
    let mut tasks = vec![task(1, TaskState::READY, &[], None)];
    for n in 1..=3 {
        let owner = format!("p{n}");
        tasks.push(task(10 + n, TaskState::SUBMITTED, &[], Some(&owner)));
    }
    let board = state(tasks, &[]);
    let released = [by("released", json!({"task": 1}), "p1")];
    assert_eq!(
        woken(&run, "p1", &members, &released, &board),
        ["p2", "p3", "parent"]
    );
    assert_eq!(
        woken(&run, "", &members, &released, &board),
        ["p2", "p3", "parent"]
    );
}

#[test]
fn a_claim_that_leaves_ready_work_wakes_members_free_to_take_it() {
    let run = run();
    let members = live(&["parent", "w1", "p1"]);
    let mut tasks = vec![
        task(1, TaskState::CLAIMED, &[], Some("w1")),
        task(2, TaskState::READY, &[], None),
        task(3, TaskState::SUBMITTED, &[], Some("p1")),
    ];
    let claimed = [event("claimed", json!({"task": 1}))];
    assert_eq!(
        woken(&run, "w1", &members, &claimed, &state(tasks.clone(), &[])),
        ["p1"]
    );
    tasks[1].status = TaskState::CLAIMED;
    assert_eq!(
        woken(&run, "w1", &members, &claimed, &state(tasks, &[])),
        NOBODY
    );
}

#[test]
fn work_created_while_all_were_busy_goes_to_the_first_to_submit() {
    let run = run();
    let members = live(&["parent", "a", "b"]);
    let board = state(
        vec![
            task(1, TaskState::READY, &[], None),
            task(2, TaskState::SUBMITTED, &[], Some("a")),
            task(3, TaskState::SUBMITTED, &[], Some("b")),
        ],
        &[],
    );
    let submitted = [event("submitted", json!({"task": 2}))];
    assert_eq!(
        woken(&run, "a", &members, &submitted, &board),
        ["b", "parent"]
    );
}

#[test]
fn ready_work_with_unmet_dependencies_wakes_nobody() {
    let run = run();
    let members = live(&["parent", "free"]);
    let board = state(
        vec![
            task(1, TaskState::CLAIMED, &[], Some("parent")),
            task(2, TaskState::READY, &[1], None),
        ],
        &[],
    );
    let created = [event("task_created", json!({"task": 2}))];
    assert_eq!(woken(&run, "parent", &members, &created, &board), NOBODY);
    // A dependency on a task the board does not hold is unmet too.
    let board = state(vec![task(2, TaskState::READY, &[7], None)], &[]);
    assert_eq!(woken(&run, "parent", &members, &created, &board), NOBODY);
    // Met dependencies release the work.
    let board = state(
        vec![
            task(1, TaskState::COMPLETED, &[], Some("free")),
            task(2, TaskState::READY, &[1], None),
        ],
        &[],
    );
    assert_eq!(woken(&run, "parent", &members, &created, &board), ["free"]);
}

#[test]
fn a_confirmed_death_re_offers_ready_work() {
    let run = run();
    let members = vec![
        member("parent", MemberState::LIVE),
        member("gone", MemberState::DEAD),
        member("parked", MemberState::LIVE),
    ];
    let board = state(
        vec![
            task(1, TaskState::READY, &[], None),
            task(2, TaskState::SUBMITTED, &[], Some("parked")),
        ],
        &[],
    );
    let death = [by("death_confirmed", json!({"member": "gone"}), "parent")];
    assert_eq!(woken(&run, "parent", &members, &death, &board), ["parked"]);
}

#[test]
fn the_fallback_never_wakes_a_busy_worker() {
    let run = run();
    let members = live(&["parent", "busy"]);
    let board = state(
        vec![
            task(1, TaskState::READY, &[], None),
            task(2, TaskState::CLAIMED, &[], Some("busy")),
        ],
        &[],
    );
    let created = [event("task_created", json!({"task": 1}))];
    assert_eq!(woken(&run, "parent", &members, &created, &board), NOBODY);
}

#[test]
fn an_amended_contract_wakes_every_live_member_including_parked() {
    let run = run();
    let members = live(&["parent", "free", "parked"]);
    let board = state(
        vec![task(2, TaskState::SUBMITTED, &[], Some("parked"))],
        &[],
    );
    let amended = [event("amended", json!({}))];
    assert_eq!(
        woken(&run, "parent", &members, &amended, &board),
        ["free", "parked"]
    );
}

#[test]
fn event_actor_overrides_the_caller_actor() {
    // The caller `free` is not the event's actor, so it is a taker; the
    // event's actor `parked` is not, although the caller would have been.
    let run = run();
    let members = live(&["parent", "free", "parked"]);
    let board = state(vec![task(1, TaskState::READY, &[], None)], &[]);
    let created = [by("task_created", json!({"task": 1}), "parked")];
    assert_eq!(woken(&run, "parent", &members, &created, &board), ["free"]);
    // Without an event actor the caller's view applies: `parked` is free.
    let created = [event("task_created", json!({"task": 1}))];
    assert_eq!(
        woken(&run, "parent", &members, &created, &board),
        ["free", "parked"]
    );
    // Whatever the event actor, the caller itself is never woken.
    let created = [by("task_created", json!({"task": 1}), "parent")];
    assert_eq!(woken(&run, "free", &members, &created, &board), ["parked"]);
}

#[test]
fn message_accepted_wakes_only_while_unread() {
    let run = run();
    let members = live(&["parent", "worker", "other"]);
    let accepted = [event(
        "message_accepted",
        json!({"message": 3, "recipient": "other"}),
    )];
    assert_eq!(
        woken(
            &run,
            "worker",
            &members,
            &accepted,
            &state(Vec::new(), &[3])
        ),
        ["other"]
    );
    assert_eq!(
        woken(
            &run,
            "worker",
            &members,
            &accepted,
            &state(Vec::new(), &[4])
        ),
        NOBODY
    );
    // A string id is not the integer id: no match.
    let textual = [event(
        "message_accepted",
        json!({"message": "3", "recipient": "other"}),
    )];
    assert_eq!(
        woken(&run, "worker", &members, &textual, &state(Vec::new(), &[3])),
        NOBODY
    );
    // The sender is never woken by its own message, nor is a dead recipient.
    let own = [event(
        "message_accepted",
        json!({"message": 3, "recipient": "worker"}),
    )];
    assert_eq!(
        woken(&run, "worker", &members, &own, &state(Vec::new(), &[3])),
        NOBODY
    );
    let dead = vec![
        member("parent", MemberState::LIVE),
        member("other", MemberState::DEAD),
    ];
    assert_eq!(
        woken(&run, "worker", &dead, &accepted, &state(Vec::new(), &[3])),
        NOBODY
    );
}

#[test]
fn submitted_or_blocked_wakes_the_coordinator_only_if_the_task_still_has_that_status() {
    let run = run();
    let members = live(&["parent", "worker"]);
    for (action, status, other) in [
        ("submitted", TaskState::SUBMITTED, TaskState::BLOCKED),
        ("blocked", TaskState::BLOCKED, TaskState::SUBMITTED),
    ] {
        let events = [event(action, json!({"task": 1}))];
        let current = state(vec![task(1, status, &[], Some("worker"))], &[]);
        assert_eq!(
            woken(&run, "worker", &members, &events, &current),
            ["parent"],
            "{action}"
        );
        let moved = state(vec![task(1, other, &[], Some("worker"))], &[]);
        assert_eq!(
            woken(&run, "worker", &members, &events, &moved),
            NOBODY,
            "{action}"
        );
        let missing = state(Vec::new(), &[]);
        assert_eq!(
            woken(&run, "worker", &members, &events, &missing),
            NOBODY,
            "{action}"
        );
    }
}

#[test]
fn evidence_and_death_wake_the_coordinator_unless_it_is_the_actor() {
    let run = run();
    let members = live(&["parent", "worker"]);
    let board = state(Vec::new(), &[]);
    for action in ["evidence", "death_confirmed"] {
        let events = [event(action, json!({}))];
        assert_eq!(
            woken(&run, "worker", &members, &events, &board),
            ["parent"],
            "{action}"
        );
        assert_eq!(
            woken(&run, "parent", &members, &events, &board),
            NOBODY,
            "{action}"
        );
    }
}

#[test]
fn every_status_but_running_wakes_nobody() {
    let members = live(&["parent", "worker"]);
    let events = [event("amended", json!({}))];
    let board = state(Vec::new(), &[]);
    for status in [
        RunState::SETUP,
        RunState::PAUSED,
        RunState::SUCCEEDED,
        RunState::BLOCKED,
        RunState::FAILED,
        RunState::CANCELLED,
        RunState::BUDGET_EXHAUSTED,
        RunState::new("future-status"),
    ] {
        let run = RunRecord {
            status: status.clone(),
            ..run()
        };
        assert_eq!(
            woken(&run, "parent", &members, &events, &board),
            NOBODY,
            "{status:?}"
        );
    }
    assert_eq!(
        woken(&run(), "parent", &members, &events, &board),
        ["worker"]
    );
}

#[test]
fn a_reserved_member_is_never_woken() {
    let run = run();
    let members = vec![
        member("parent", MemberState::LIVE),
        member("new", MemberState::RESERVED),
    ];
    let board = state(vec![task(1, TaskState::READY, &[], None)], &[]);
    let events = [
        event("amended", json!({})),
        event("task_created", json!({"task": 1})),
    ];
    assert_eq!(woken(&run, "parent", &members, &events, &board), NOBODY);
}

#[test]
fn ready_work_takers_returns_the_free_set_including_the_coordinator() {
    // Python's `free - {coordinator}` decides, then `free` is returned whole.
    let run = run();
    let everyone: BTreeSet<String> = ["parent", "free", "busy"].map(String::from).into();
    let busy = task(1, TaskState::CLAIMED, &[], Some("busy"));
    let tasks: BTreeMap<i64, &TaskSummary> = [(1, &busy)].into();
    let takers = ready_work_takers(&run, "busy", &everyone, &tasks);
    assert_eq!(takers, ["free", "parent"].map(String::from).into());
    // Only the coordinator free: the fallback excludes working owners only.
    let takers = ready_work_takers(&run, "free", &everyone, &tasks);
    assert_eq!(takers, ["parent"].map(String::from).into());
    // An empty owner holds nothing.
    let unowned = task(2, TaskState::CLAIMED, &[], Some(""));
    let tasks: BTreeMap<i64, &TaskSummary> = [(2, &unowned)].into();
    let takers = ready_work_takers(&run, "parent", &everyone, &tasks);
    assert_eq!(takers, ["busy", "free"].map(String::from).into());
}

#[test]
fn the_action_tables_match_python() {
    assert_eq!(
        READY_WORK_ACTIONS,
        [
            "task_created",
            "dependencies",
            "released",
            "verified",
            "revalidated",
            "recovered",
            "revoked",
            "claimed",
            "submitted",
            "blocked",
            "death_confirmed",
        ]
    );
    assert_eq!(OWNERSHIP_ACTIONS, ["claimed", "submitted", "blocked"]);
    assert_eq!(WORK_HOLDING_STATUSES, ["claimed", "blocked", "submitted"]);
    assert_eq!(WORKING_STATUSES, ["claimed", "blocked"]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "carries its key")]
fn a_keyed_action_without_its_key_is_a_debug_assertion() {
    let events = [event("submitted", json!({}))];
    let _ = notification_targets(
        &run(),
        "worker",
        &live(&["parent", "worker"]),
        &events,
        &state(Vec::new(), &[]),
    );
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
