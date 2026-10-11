use super::*;
use crate::domain::project_board::services::claims::take_over;
use crate::domain::project_board::services::fixtures_tests::{at, open_run, person, plan, task};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{ItemState, TaskStatus::*};

fn item(id: &str) -> RunItem {
    RunItem {
        id: Slug::parse(id).unwrap(),
        state: ItemState::Done,
        owner: None,
        evidence: vec!["PR #7".into()],
    }
}

fn ended(run: &Run) -> Run {
    lost_at(run, &at("09:30:00")).unwrap()
}

#[test]
fn a_run_begins_on_a_held_task_from_its_accepted_plan_one_at_a_time() {
    let accepted = plan(1, PlanState::Accepted);
    let first = begin_run(&task(InProgress), &[], &accepted, &at("10:00:00")).unwrap();
    assert_eq!(
        (
            first.fields().number,
            first.fields().plan_version,
            first.is_open()
        ),
        (1, 1, true)
    );
    assert_eq!(
        begin_run(
            &task(InProgress),
            std::slice::from_ref(&first),
            &accepted,
            &at("10:00:00")
        ),
        Err(BoardRuleError::RunOpen(1)),
        "at most one run going"
    );
    let second = begin_run(
        &task(InProgress),
        &[ended(&first)],
        &accepted,
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(second.fields().number, 2);
    for status in [Draft, Ready, Done, Archived] {
        assert_eq!(
            begin_run(&task(status), &[], &accepted, &at("10:00:00")),
            Err(BoardRuleError::NotHeld),
            "{status:?}"
        );
    }
    for plan in [plan(1, PlanState::Draft), plan(2, PlanState::Accepted)] {
        assert_eq!(
            begin_run(&task(InProgress), &[], &plan, &at("10:00:00")),
            Err(BoardRuleError::NotThisTasks),
            "only the task's accepted plan runs"
        );
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

/// The error names what is wrong: an item the plan does not have, a plan
/// of another version, or one never accepted.
#[test]
fn a_fold_back_reports_only_its_own_accepted_plans_items() {
    let accepted = plan(1, PlanState::Accepted);
    let field = |result: Result<Run, BoardRuleError>| match result {
        Err(BoardRuleError::Invalid(error)) => error.field,
        other => panic!("{other:?}"),
    };
    let fold = |run: &Run, plan: &Plan, items: Vec<RunItem>| {
        fold_back(run, plan, RunOutcome::Failed, None, items, &at("09:30:00"))
    };
    assert_eq!(
        field(fold(&open_run(1, 1), &accepted, vec![item("ghost")])),
        "items/0/id"
    );
    assert_eq!(
        field(fold(&open_run(1, 2), &accepted, vec![])),
        "plan_version"
    );
    for state in [PlanState::Draft, PlanState::Discarded] {
        assert_eq!(
            field(fold(&open_run(1, 1), &plan(1, state), vec![])),
            "plan"
        );
    }
    assert!(fold(&open_run(1, 1), &plan(1, PlanState::Superseded), vec![]).is_ok());
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
    for status in [Claimed, InProgress, Review, Blocked] {
        let ended = end_lost(
            &task(status),
            &open_run(1, 1),
            &person("Ada"),
            &at("10:00:00"),
        )
        .unwrap();
        assert_eq!(ended.fields().outcome, Some(RunOutcome::Lost), "{status:?}");
        assert_eq!(
            end_lost(
                &task(status),
                &open_run(1, 1),
                &person("Bob"),
                &at("10:00:00")
            ),
            Err(BoardRuleError::NotHolder)
        );
    }
    let taken = take_over(&task(InProgress), &person("Bob"), &at("11:30:00")).unwrap();
    assert!(
        end_lost(&taken, &open_run(1, 1), &person("Bob"), &at("11:31:00")).is_ok(),
        "whoever takes an expired claim over ends its run"
    );
    let mut elsewhere = open_run(1, 1).into_fields();
    elsewhere.task = Slug::parse("other").unwrap();
    assert_eq!(
        end_lost(
            &task(InProgress),
            &Run::new(elsewhere).unwrap(),
            &person("Ada"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::NotThisTasks)
    );
}

#[test]
fn a_task_leaves_its_holder_only_once_its_run_has_ended() {
    let held = task(InProgress);
    let running = open_run(1, 1);
    for to in [Ready, Done, Archived] {
        assert_eq!(
            may_move_with(&held, Some(&running), to),
            Err(BoardRuleError::RunOpen(1))
        );
    }
    for to in [Claimed, InProgress, Review, Blocked] {
        assert_eq!(may_move_with(&held, Some(&running), to), Ok(()));
    }
    assert_eq!(may_move_with(&held, Some(&ended(&running)), Ready), Ok(()));
    assert_eq!(may_move_with(&held, None, Ready), Ok(()));
}
