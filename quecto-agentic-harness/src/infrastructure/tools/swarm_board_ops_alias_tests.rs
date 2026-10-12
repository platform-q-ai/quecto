//! #2478: the swarm's tasks are task items. The tool takes each op's and
//! argument's task-item name beside its old one, and reads it as the old
//! one before anything else: binding, the running gate, the board call,
//! refusals and every `swarm_op` record see only the old names, so the
//! board, its events and the golden fixtures are unchanged.
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Recorded, board, board_with, deadline, direct, execute, masked, prepared, refused};
use crate::domain::swarm::{
    ArgumentFaults, BoardOpObservation, BoardOpOutcome, RefusalKind, UnexpectedArgs,
};
use crate::infrastructure::tools::swarm_board_ops::{
    BOARD_OPS, HARNESS_OPS, ID_ALIAS, OP_ALIASES, canonical_op, tool_schema,
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

/// `task_id` and `task_item_id` are one argument: both, with one value,
/// are taken; with two, the call is refused before the board.
#[tokio::test]
async fn task_id_and_task_item_id_disagreeing_are_refused() {
    let (_directory, context) = board();
    direct(&context, "task_create", json!(["r1", "a", ["pass"]])).unwrap();
    direct(&context, "task_create", json!(["r2", "b", ["pass"]])).unwrap();
    let both = execute(
        &context,
        &json!({"op": "task", "task_id": 1, "task_item_id": 1}).to_string(),
    )
    .await;
    assert!(!both.is_error, "{}", both.content);
    let refusal = refused(
        &context,
        &json!({"op": "claim", "task_id": 1, "task_item_id": 2}).to_string(),
    )
    .await;
    assert!(
        refusal.contains("task_id") && refusal.contains("task_item_id"),
        "{refusal}"
    );
    // Nothing was claimed.
    let row = direct(&context, "task", json!([1])).unwrap();
    assert_eq!(row["status"], "ready", "{row}");
}

/// A coordinator's board recording in its event log.
fn logged() -> (
    tempfile::TempDir,
    crate::infrastructure::tools::swarm_bridge::SwarmContext,
    Arc<Recorded>,
) {
    let (directory, context) = board();
    let log = Arc::new(Recorded::default());
    assert!(context.board.record_in(log.clone()));
    (directory, context, log)
}

/// The `swarm_op` records and the board's events keep the old names: an
/// op called by its task-item name is recorded as its old one, and an
/// argument fault names `task_id`, not `task_item_id`.
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
    // `task_create` takes no task id: the fault names `task_id`.
    refused(
        &context,
        &json!({"op": "task_item_create", "request": "r2", "title": "b",
            "acceptance": ["pass"], "task_item_id": 1})
        .to_string(),
    )
    .await;
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
                known: vec!["task_id".to_owned()],
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
    assert!(actions.contains(&"task_created"), "{actions:?}");
    assert!(
        actions.iter().all(|action| !action.contains("item")),
        "{actions:?}"
    );
}

/// The schema takes either name: its op enum lists each task-item name
/// beside the old ones, and `task_item_id` has `task_id`'s schema. The
/// valid-ops guidance still lists only the old names (slice B changes
/// what agents read).
#[test]
fn the_schema_takes_either_name_and_the_guidance_is_unchanged() {
    let schema = tool_schema();
    let ops: Vec<&str> = schema["properties"]["op"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op.as_str().unwrap())
        .collect();
    let old: Vec<&str> = HARNESS_OPS
        .iter()
        .copied()
        .chain(BOARD_OPS.iter().map(|spec| spec.name))
        .collect();
    let aliases: Vec<&str> = OP_ALIASES.iter().map(|(alias, _)| *alias).collect();
    assert_eq!(ops, [old.clone(), aliases].concat());
    assert_eq!(
        schema["properties"][ID_ALIAS.0],
        schema["properties"][ID_ALIAS.1]
    );
    assert_eq!(
        crate::infrastructure::tools::swarm::swarm_guidance::valid_ops(),
        old
    );
}
