use super::*;
use crate::domain::project_board::entities::run::RunItem;
use crate::domain::project_board::services::claims::take_over;
use crate::domain::project_board::services::fixtures_tests::{at, open_run, person, plan, task};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{
    ItemState, PlanState, TaskStatus::*,
};

fn item(id: &str) -> RunItem {
    RunItem {
        id: Slug::parse(id).unwrap(),
        state: ItemState::Done,
        owner: None,
        evidence: vec!["PR #7".into()],
    }
}

#[test]
fn a_run_folds_back_once_with_its_outcome_cost_and_items() {
    let accepted = plan(1, PlanState::Accepted);
    let folded = fold_back(
        &open_run(1, 1),
        &accepted,
        RunOutcome::Succeeded,
        Some(500),
        vec![item("schema"), item("api")],
        &at("09:30:00"),
    )
    .unwrap();
    let fields = folded.fields();
    assert_eq!(
        (
            &fields.ended,
            fields.outcome,
            fields.cost,
            fields.items.len()
        ),
        (
            &Some(at("09:30:00")),
            Some(RunOutcome::Succeeded),
            Some(500),
            2
        )
    );
    assert_eq!(
        fold_back(
            &folded,
            &accepted,
            RunOutcome::Failed,
            None,
            vec![],
            &at("09:40:00")
        ),
        Err(BoardRuleError::RunEnded(1))
    );
}

#[test]
fn a_fold_back_reports_only_its_own_plans_items() {
    let accepted = plan(1, PlanState::Accepted);
    assert_eq!(
        fold_back(
            &open_run(1, 1),
            &accepted,
            RunOutcome::Failed,
            None,
            vec![item("ghost")],
            &at("09:30:00")
        ),
        Err(BoardRuleError::NotThisTasks)
    );
    assert_eq!(
        fold_back(
            &open_run(1, 2),
            &accepted,
            RunOutcome::Failed,
            None,
            vec![],
            &at("09:30:00")
        ),
        Err(BoardRuleError::NotThisTasks),
        "the run ran version 2"
    );
}

#[test]
fn a_run_ends_no_earlier_than_it_started() {
    let folded = fold_back(
        &open_run(1, 1),
        &plan(1, PlanState::Accepted),
        RunOutcome::Stopped,
        None,
        vec![],
        &at("08:00:00"),
    )
    .unwrap();
    assert_eq!(folded.fields().ended, Some(at("09:00:00")));
}

#[test]
fn the_holder_ends_an_open_run_as_lost_and_others_may_not() {
    let accepted = plan(1, PlanState::Accepted);
    for status in [Claimed, InProgress, Review, Blocked] {
        let ended = end_lost(
            &task(status),
            &open_run(1, 1),
            &accepted,
            &person("Ada"),
            &at("10:00:00"),
        )
        .unwrap();
        assert_eq!(ended.fields().outcome, Some(RunOutcome::Lost), "{status:?}");
        assert_eq!(
            end_lost(
                &task(status),
                &open_run(1, 1),
                &accepted,
                &person("Bob"),
                &at("10:00:00")
            ),
            Err(BoardRuleError::NotHolder)
        );
    }
    let taken = take_over(&task(InProgress), &person("Bob"), &at("11:30:00")).unwrap();
    assert!(
        end_lost(
            &taken,
            &open_run(1, 1),
            &accepted,
            &person("Bob"),
            &at("11:31:00")
        )
        .is_ok(),
        "whoever takes an expired claim over ends its run"
    );
}

#[test]
fn a_task_leaves_its_holder_only_once_its_run_has_ended() {
    let running = open_run(1, 1);
    for to in [Ready, Done, Archived] {
        assert_eq!(
            may_move_with(Some(&running), to),
            Err(BoardRuleError::RunOpen(1))
        );
    }
    for to in [Claimed, InProgress, Review, Blocked] {
        assert_eq!(may_move_with(Some(&running), to), Ok(()));
    }
    let ended = end_lost(
        &task(InProgress),
        &running,
        &plan(1, PlanState::Accepted),
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(may_move_with(Some(&ended), Ready), Ok(()));
    assert_eq!(may_move_with(None, Ready), Ok(()));
}
