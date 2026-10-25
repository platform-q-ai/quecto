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

/// #2705 T5: a run lost mid-claim (its host died while its holder's claim
/// was live) moves its task back to ready, the claim released with it: a
/// loss is no outcome its holder keeps the task for.
#[test]
fn a_run_lost_mid_claim_moves_its_task_back_to_ready_releasing_the_claim() {
    use crate::domain::project_board::services::run_outcomes::{StopKind, task_after_end};
    let held = task(InProgress);
    let run = lost(&held, &open_run(1, 1), &person("Ada"), &at("10:00:00")).unwrap();
    let moved = task_after_end(&held, &run, StopKind::Cancelled, &at("10:00:00"))
        .unwrap()
        .expect("a loss moves its task");
    let fields = moved.fields();
    assert_eq!((fields.status, fields.claim.clone()), (Ready, None));
    assert_eq!(
        fields.updated,
        run.fields().ended.clone().unwrap(),
        "one stamp"
    );
}

/// One whose claim lapsed is taken over first, then lost: its taker's
/// claim goes with the move back to ready too.
#[test]
fn a_run_lost_after_a_take_over_releases_the_takers_claim() {
    use crate::domain::project_board::services::run_outcomes::{StopKind, task_after_end};
    let bob = person("Bob");
    let taken = take_over(&task(InProgress), &bob, &at("11:30:00")).unwrap();
    let run = lost(&taken, &open_run(1, 1), &bob, &at("11:31:00")).unwrap();
    let moved = task_after_end(&taken, &run, StopKind::Cancelled, &at("11:31:00"))
        .unwrap()
        .expect("a loss moves its task");
    let fields = moved.fields();
    assert_eq!((fields.status, fields.claim.clone()), (Ready, None));
}
