//! #2657: a run ended as lost recorded no task items, though a stopped
//! one recorded them as todo and a worker had finished an item before the
//! container went. A lost run records what is known, as a held end does:
//! where each of its plan's task items stood (todo when no board was
//! kept), and its cost when known.
use super::*;

#[test]
fn a_lost_run_records_its_plans_task_items_as_reported() {
    let held = task(InProgress);
    let accepted = plan(1, PlanState::Accepted);
    let todo = RunItem {
        state: ItemState::Todo,
        evidence: vec![],
        ..item("api")
    };
    let reported = Reported {
        items: vec![item("schema"), todo.clone()],
        cost: Some(9_000),
        report: SwarmReport::Kept,
        unpushed: 0,
    };
    let ended = end_lost(
        &held,
        (&open_run(1, 1), &accepted),
        reported,
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap();
    let fields = ended.fields();
    assert_eq!(fields.outcome, Some(RunOutcome::Lost));
    assert_eq!(
        fields.items,
        [item("schema"), todo],
        "where each item stood"
    );
    assert_eq!(fields.cost, Some(9_000), "its cost as reported");
    assert_eq!(fields.ended, Some(at("10:00:00")));
}

#[test]
fn a_lost_run_is_refused_items_its_plan_does_not_have() {
    let reported = Reported {
        items: vec![item("schema"), item("api"), item("stray")],
        cost: None,
        report: SwarmReport::Kept,
        unpushed: 0,
    };
    assert!(matches!(
        end_lost(
            &task(InProgress),
            (&open_run(1, 1), &plan(1, PlanState::Accepted)),
            reported,
            &person("Ada"),
            &at("10:00:00"),
        ),
        Err(BoardRuleError::Invalid(_))
    ));
}
