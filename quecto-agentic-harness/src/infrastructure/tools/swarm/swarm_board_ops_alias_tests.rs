//! #2478: the swarm's tasks are task items. The tool takes each op's and
//! argument's task-item name beside its old one, and reads it as the old
//! one before anything else: binding, the running gate, the board call,
//! refusals and every `swarm_op` record see only the old names, so the
//! board, its events and the golden fixtures are unchanged.
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Recorded, board, board_with, deadline, direct, execute, masked, prepared, refused};
use crate::domain::swarm::{
    ArgumentFaults, BoardOpObservation, BoardOpOutcome, RefusalKind, UnexpectedArgs, WrongTypeArg,
};
use crate::infrastructure::tools::swarm::swarm_board_ops::{
    BOARD_OPS, HARNESS_OPS, ID_ALIAS, OP_ALIASES, canonical_op,
};

/// `request` with each old name (the op's, `task_id`) under its
/// task-item name, where it has one.
fn task_item_names(mut request: Value) -> Value {
    let fields = request.as_object_mut().unwrap();
    let op = fields["op"].as_str().unwrap().to_owned();
    if let Some((alias, _)) = OP_ALIASES.iter().find(|(_, old)| *old == op) {
        fields.insert("op".into(), json!(alias));
    }
    if let Some(id) = fields.shift_remove(ID_ALIAS.1) {
        fields.insert(ID_ALIAS.0.into(), id);
    }
    request
}

#[test]
fn the_task_item_names_are_the_ones_asked_for() {
    assert_eq!(
        OP_ALIASES,
        &[
            ("task_item", "task"),
            ("task_items", "tasks"),
            ("task_item_create", "task_create"),
            ("verify_task_item", "verify_task"),
            ("revalidate_task_item", "revalidate_task"),
        ]
    );
    assert_eq!(ID_ALIAS, ("task_item_id", "task_id"));
    for (alias, old) in OP_ALIASES {
        assert_eq!(canonical_op(alias), *old);
        assert!(BOARD_OPS.iter().any(|spec| spec.name == *old), "{old}");
    }
    assert_eq!(canonical_op("claim"), "claim");
    // An alias is no op's own name, and names one op only.
    let names: Vec<&str> = OP_ALIASES.iter().map(|(alias, _)| *alias).collect();
    for alias in &names {
        assert!(!HARNESS_OPS.contains(alias), "{alias}");
        assert!(BOARD_OPS.iter().all(|spec| spec.name != *alias), "{alias}");
        assert_eq!(names.iter().filter(|name| *name == alias).count(), 1);
    }
    assert!(BOARD_OPS.iter().all(|spec| spec.name != ID_ALIAS.0));
}

/// Each op under its task-item names on one board, under its old names on
/// an identical one: equal answers, or equal refusal texts.
#[tokio::test]
async fn every_task_item_name_reaches_the_op_of_its_old_name() {
    let (mut aliased_ops, mut aliased_ids) = (0, 0);
    for spec in BOARD_OPS {
        let takes_id = spec.args.iter().any(|arg| arg.name == ID_ALIAS.1);
        let aliased = OP_ALIASES.iter().any(|(_, old)| *old == spec.name);
        if !takes_id && !aliased {
            continue;
        }
        aliased_ops += usize::from(aliased);
        aliased_ids += usize::from(takes_id);
        let lifecycle = || Arc::new(crate::application::swarm::LifecycleService);
        let deadline = deadline();
        let (_old_dir, old_board) = board_with(deadline, lifecycle());
        let (_new_dir, new_board) = board_with(deadline, lifecycle());
        let mut old = prepared(spec.name, &old_board);
        old["op"] = json!(spec.name);
        let mut new = prepared(spec.name, &new_board);
        new["op"] = json!(spec.name);
        let new = task_item_names(new);
        let by_old = execute(&old_board, &old.to_string()).await;
        let by_new = execute(&new_board, &new.to_string()).await;
        assert_eq!(
            by_new.is_error, by_old.is_error,
            "{new}: {}",
            by_new.content
        );
        match by_old.is_error {
            true => assert_eq!(by_new.content, by_old.content, "{new}"),
            false => {
                let parsed = |text: &str| masked(serde_json::from_str::<Value>(text).unwrap());
                assert_eq!(parsed(&by_new.content), parsed(&by_old.content), "{new}");
            }
        }
    }
    assert_eq!((aliased_ops, aliased_ids), (5, 13));
}

/// A coordinator's board recording in its event log.
fn logged() -> (
    tempfile::TempDir,
    crate::infrastructure::tools::swarm::swarm_bridge::SwarmContext,
    Arc<Recorded>,
) {
    let (directory, context) = board();
    let log = Arc::new(Recorded::default());
    assert!(context.board.record_in(log.clone()));
    (directory, context, log)
}

/// The refusal of `claim` given `task_id` and `task_item_id` with two
/// values.
pub(super) const CONFLICT: &str = "tool error: swarm: \"claim: task_id and task_item_id name the same argument but hold different values; send only one\"";

/// `task_id` and `task_item_id` are one argument: both, with one value,
/// are taken; with two (`1` and `2`, `1` and `1.0`, `null` and `1`), the
/// binding refuses the call as Python refuses an argument given twice,
/// after the running gate as every binding refusal: `calling`, its fault
/// naming `task_id` (and, for `null`, the type `task_id` was given), and
/// nothing claimed.
#[tokio::test]
async fn task_id_and_task_item_id_disagreeing_are_refused() {
    let (_directory, context, log) = logged();
    direct(&context, "task_create", json!(["r1", "a", ["pass"]])).unwrap();
    direct(&context, "task_create", json!(["r2", "b", ["pass"]])).unwrap();
    let both = execute(
        &context,
        &json!({"op": "task", "task_id": 1, "task_item_id": 1}).to_string(),
    )
    .await;
    assert!(!both.is_error, "{}", both.content);
    log.take();
    let given_twice = UnexpectedArgs {
        count: 1,
        known: vec!["task_id".to_owned()],
    };
    let null_for_integer = WrongTypeArg {
        arg: "task_id".to_owned(),
        expected: "integer".to_owned(),
    };
    for (old, new, wrong_type_args) in [
        ("1", "2", vec![]),
        ("1", "1.0", vec![]),
        ("null", "1", vec![null_for_integer]),
    ] {
        let text = format!(r#"{{"op":"claim","task_id":{old},"task_item_id":{new}}}"#);
        assert_eq!(refused(&context, &text).await, CONFLICT, "{text}");
        let records = log.take();
        let claims: Vec<&BoardOpObservation> = records
            .iter()
            .filter(|record| record.op == "claim")
            .collect();
        assert_eq!(claims.len(), 1, "{text}: {records:?}");
        let record = claims[0];
        assert_eq!(
            record.outcome,
            BoardOpOutcome::Refused {
                kind: RefusalKind::Calling,
                committed: false,
            },
            "{text}"
        );
        assert_eq!(
            *record.arguments,
            ArgumentFaults {
                unexpected_args: Some(given_twice.clone()),
                wrong_type_args,
                ..ArgumentFaults::NONE
            },
            "{text}"
        );
    }
    // Neither task was claimed.
    for id in [1, 2] {
        let row = direct(&context, "task", json!([id])).unwrap();
        assert_eq!(row["status"], "ready", "{row}");
    }
}

/// A conflict meets the running gate first, as every binding refusal does:
/// on a paused run it is refused as not running, with no argument fault.
#[tokio::test]
async fn a_conflict_on_a_paused_run_meets_the_running_gate_first() {
    let (_directory, context, log) = logged();
    direct(&context, "task_create", json!(["r1", "a", ["pass"]])).unwrap();
    crate::infrastructure::tools::registry::call_work::off_the_runtime(|| context.pause("hold"))
        .unwrap();
    log.take();
    let text = r#"{"op":"claim","task_id":1,"task_item_id":2}"#;
    let refusal = refused(&context, text).await;
    assert!(
        refusal.contains("the swarm run is paused, so op=claim is unavailable"),
        "{refusal}"
    );
    let records = log.take();
    let claims: Vec<&BoardOpObservation> = records
        .iter()
        .filter(|record| record.op == "claim")
        .collect();
    assert_eq!(claims.len(), 1, "{records:?}");
    assert_eq!(
        claims[0].outcome,
        BoardOpOutcome::Refused {
            kind: RefusalKind::NotRunning,
            committed: false,
        }
    );
    assert_eq!(*claims[0].arguments, ArgumentFaults::NONE);
    assert_eq!(
        claims[0].name_form,
        Some(crate::domain::swarm::NameForm::Legacy)
    );
}

/// The `swarm_op` records and the board's events keep the old names: an
/// op called by its task-item name is recorded as its old one; an argument
/// sent to an op that does not take it is named as it was sent.
#[tokio::test]
async fn the_records_and_events_keep_the_old_names() {
    let (_directory, context, log) = logged();
    let created = execute(
        &context,
        &json!({"op": "task_item_create", "request": "r1", "title": "a",
            "acceptance": ["pass"]})
        .to_string(),
    )
    .await;
    assert!(!created.is_error, "{}", created.content);
    let claimed = execute(
        &context,
        &json!({"op": "claim", "task_item_id": 1}).to_string(),
    )
    .await;
    assert!(!claimed.is_error, "{}", claimed.content);
    // `task_create` takes no task id: the refusal and its fault name the
    // argument as it was sent.
    let wrong_op = refused(
        &context,
        &json!({"op": "task_item_create", "request": "r2", "title": "b",
            "acceptance": ["pass"], "task_item_id": 1})
        .to_string(),
    )
    .await;
    assert_eq!(
        wrong_op,
        r#"tool error: swarm: "task_item_create: unexpected argument task_item_id""#
    );
    let records: Vec<BoardOpObservation> = log.take();
    // The harness's own reads (`_status`, `summary`, ...) are left out.
    let calls: Vec<&BoardOpObservation> = records
        .iter()
        .filter(|record| ["task_create", "claim"].contains(&record.op.as_str()))
        .collect();
    let ops: Vec<&str> = calls.iter().map(|record| record.op.as_str()).collect();
    assert_eq!(ops, ["task_create", "claim", "task_create"]);
    assert!(
        records.iter().all(|record| !record.op.contains("item")),
        "{records:?}"
    );
    let refusal = calls.last().expect("the refused op is recorded");
    assert!(
        matches!(
            refusal.outcome,
            BoardOpOutcome::Refused {
                kind: RefusalKind::Calling,
                ..
            }
        ),
        "{:?}",
        refusal.outcome
    );
    assert_eq!(
        *refusal.arguments,
        ArgumentFaults {
            unexpected_args: Some(UnexpectedArgs {
                count: 1,
                known: vec!["task_item_id".to_owned()],
            }),
            ..ArgumentFaults::NONE
        }
    );
    let events = direct(&context, "events", json!([0, 50])).unwrap();
    let actions: Vec<&str> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["action"].as_str())
        .collect();
    assert_eq!(
        actions
            .iter()
            .filter(|action| **action == "task_created")
            .count(),
        1,
        "{actions:?}"
    );
}

/// A `task_item_id` value the board cannot hold is recorded under the
/// argument it binds as, `task_id`, for an op that takes it; under its
/// own name for one that does not.
#[tokio::test]
async fn an_unreadable_task_item_id_is_recorded_as_it_binds() {
    let (_directory, context, log) = logged();
    for (text, op, field) in [
        (
            r#"{"op":"task_item","task_item_id":1e400}"#,
            "task",
            "task_id",
        ),
        (
            r#"{"op":"inbox","task_item_id":1e400}"#,
            "inbox",
            "task_item_id",
        ),
    ] {
        refused(&context, text).await;
        let records = log.take();
        let record = records
            .iter()
            .find(|record| record.op == op)
            .unwrap_or_else(|| panic!("{text}: {records:?}"));
        assert_eq!(
            *record.arguments,
            ArgumentFaults {
                unreadable_args: vec![field.to_owned()],
                ..ArgumentFaults::NONE
            },
            "{text}"
        );
    }
}

#[path = "swarm_board_ops_name_form_tests.rs"]
mod name_form;
