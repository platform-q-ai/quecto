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
    let first = begin_run(
        &task(InProgress),
        &[],
        &accepted,
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap();
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
            &person("Ada"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::RunOpen(1)),
        "at most one run going"
    );
    let second = begin_run(
        &task(InProgress),
        &[ended(&first)],
        &accepted,
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(second.fields().number, 2);
    for status in [Draft, Ready, Done, Archived] {
        assert_eq!(
            begin_run(
                &task(status),
                &[],
                &accepted,
                &person("Ada"),
                &at("10:00:00")
            ),
            Err(BoardRuleError::NoClaim),
            "{status:?}: nobody holds it"
        );
    }
    assert_eq!(
        begin_run(
            &task(InProgress),
            &[],
            &accepted,
            &person("Bob"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::NotHolder),
        "only its holder begins a run"
    );
    assert_eq!(
        begin_run(
            &task(InProgress),
            &[],
            &accepted,
            &person("Ada"),
            &at("11:00:00")
        ),
        Err(BoardRuleError::ClaimExpired)
    );
    for (plan, refused) in [
        (
            plan(1, PlanState::Draft),
            BoardRuleError::PlanNotAccepted(1),
        ),
        (
            plan(2, PlanState::Accepted),
            BoardRuleError::PlanNotAccepted(2),
        ),
    ] {
        assert_eq!(
            begin_run(
                &task(InProgress),
                &[],
                &plan,
                &person("Ada"),
                &at("10:00:00")
            ),
            Err(refused),
            "only the task's accepted plan runs"
        );
    }
    let mut elsewhere = plan(1, PlanState::Accepted).into_fields();
    elsewhere.task = Slug::parse("other").unwrap();
    assert_eq!(
        begin_run(
            &task(InProgress),
            &[],
            &Plan::new(elsewhere).unwrap(),
            &person("Ada"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::NotThisTasks)
    );
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
            may_move_with(&held, std::slice::from_ref(&running), to),
            Err(BoardRuleError::RunOpen(1))
        );
    }
    for to in [Claimed, InProgress, Review, Blocked] {
        assert_eq!(
            may_move_with(&held, std::slice::from_ref(&running), to),
            Ok(())
        );
    }
    assert_eq!(may_move_with(&held, &[ended(&running)], Ready), Ok(()));
    assert_eq!(may_move_with(&held, &[], Ready), Ok(()));
}

/// A run of a task nobody claims is never stranded: anyone ends it as
/// lost. A claimed task's run is its holder's: until 11:00 Ada's claim is
/// live and Bob can neither end the run nor take the task over; from
/// 11:00 he takes it over, and then ends the run as its holder.
#[test]
fn a_run_of_a_task_nobody_claims_is_ended_by_anyone() {
    for status in [Ready, Draft] {
        assert!(
            end_lost(
                &task(status),
                &open_run(1, 1),
                &person("Bob"),
                &at("10:00:00")
            )
            .is_ok(),
            "{status:?}: no claim"
        );
    }
    let held = task(InProgress);
    let bob = person("Bob");
    assert_eq!(
        end_lost(&held, &open_run(1, 1), &bob, &at("10:59:59")),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        take_over(&held, &bob, &at("10:59:59")),
        Err(BoardRuleError::ClaimLive)
    );
    assert_eq!(
        end_lost(&held, &open_run(1, 1), &bob, &at("11:00:00")),
        Err(BoardRuleError::NotHolder),
        "an expired claim is still Ada's until it is taken over"
    );
    let taken = take_over(&held, &bob, &at("11:00:00")).unwrap();
    assert!(end_lost(&taken, &open_run(1, 1), &bob, &at("11:00:01")).is_ok());
}

/// A run ends by the task's clock rule: never before the task's last
/// change, however far behind the caller's clock is.
#[test]
fn a_lost_run_ends_at_the_tasks_stamp() {
    let ended = end_lost(
        &task(InProgress),
        &open_run(1, 1),
        &person("Ada"),
        &at("08:00:00"),
    )
    .unwrap();
    assert_eq!(ended.fields().ended, Some(at("09:00:01")));
}

#[test]
fn another_tasks_runs_and_two_going_are_refused() {
    let mut elsewhere = open_run(1, 1).into_fields();
    elsewhere.task = Slug::parse("other").unwrap();
    let elsewhere = Run::new(elsewhere).unwrap();
    assert_eq!(
        may_move_with(&task(InProgress), std::slice::from_ref(&elsewhere), Ready),
        Err(BoardRuleError::NotThisTasks)
    );
    let both = [open_run(1, 1), open_run(2, 1)];
    assert!(matches!(
        may_move_with(&task(InProgress), &both, Ready),
        Err(BoardRuleError::RunsInconsistent(_))
    ));
    assert!(matches!(
        begin_run(
            &task(InProgress),
            &both,
            &plan(1, PlanState::Accepted),
            &person("Ada"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::RunsInconsistent(_))
    ));
}

/// A run going holds a task back from exactly the statuses that hold no
/// claim: those a task leaves its holder for.
#[test]
fn a_run_holds_moves_to_the_statuses_that_hold_no_claim() {
    for to in [
        Draft, Ready, Claimed, InProgress, Review, Blocked, Done, Archived,
    ] {
        assert_eq!(holds_moves_to(to), !to.may_hold_claim(), "{to:?}");
    }
}

#[test]
fn a_fold_back_finds_its_run_by_number_and_the_plan_it_ran_by_version() {
    let first = plan(1, PlanState::Superseded);
    let second = plan(2, PlanState::Accepted);
    let (one, two) = (open_run(1, 1), open_run(2, 2));
    let (runs, plans) = ([&one, &two], [&first, &second]);
    let (run, ran) = run_and_plan(&runs, &plans, 2).unwrap();
    assert_eq!((run, ran), (&two, &second));
    assert_eq!(run_and_plan(&runs, &plans, 1).unwrap().1, &first);
    assert_eq!(
        run_and_plan(&runs, &plans, 3),
        Err(BoardRuleError::NoSuchRun(3))
    );
    assert_eq!(
        run_and_plan(&runs, &plans[..1], 2),
        Err(BoardRuleError::UnknownPlanVersion(2))
    );
}

/// The holder of a live claim ends its task's open run as stopped or
/// failed, folded back with the items it gives; nobody else does, nor the
/// holder once the claim expired (renew first), nor any run twice.
#[test]
fn the_holder_ends_an_open_run_as_stopped_or_failed() {
    let held = task(InProgress);
    let accepted = plan(1, PlanState::Accepted);
    for (end, outcome) in [
        (HeldEnd::Stopped, RunOutcome::Stopped),
        (HeldEnd::Failed, RunOutcome::Failed),
    ] {
        let ended = end_held(
            &held,
            (&open_run(1, 1), &accepted),
            end,
            vec![],
            &person("Ada"),
            &at("10:00:00"),
        )
        .unwrap();
        assert_eq!(ended.fields().outcome, Some(outcome));
        assert_eq!(ended.fields().ended, Some(at("10:00:00")));
        assert_eq!(end.outcome(), outcome);
    }
    let end = |task: &Task, run: &Run, who: &str, when: &str| {
        end_held(
            task,
            (run, &accepted),
            HeldEnd::Failed,
            vec![],
            &person(who),
            &at(when),
        )
    };
    let running = open_run(1, 1);
    assert_eq!(
        end(&held, &running, "Bob", "10:00:00"),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        end(&held, &running, "Ada", "11:00:00"),
        Err(BoardRuleError::ClaimExpired)
    );
    assert_eq!(
        end(&task(Ready), &running, "Ada", "10:00:00"),
        Err(BoardRuleError::NoClaim)
    );
    assert_eq!(
        end(&held, &ended(&running), "Ada", "10:00:00"),
        Err(BoardRuleError::RunEnded(1))
    );
    let mut elsewhere = running.into_fields();
    elsewhere.task = Slug::parse("other").unwrap();
    assert_eq!(
        end(&held, &Run::new(elsewhere).unwrap(), "Ada", "10:00:00"),
        Err(BoardRuleError::NotThisTasks)
    );
}

/// An open run names the environment its swarm runs in, and keeps it when
/// it is folded back; an ended run, or a ref that is not one, is refused
/// (#2481 S4d).
#[test]
fn an_open_run_is_kept_in_its_swarms_environment() {
    let kept = kept_in(&open_run(1, 1), "C12").unwrap();
    assert_eq!(kept.fields().environment.as_deref(), Some("C12"));
    assert_eq!(
        RunFields {
            environment: None,
            ..kept.fields().clone()
        },
        open_run(1, 1).fields().clone()
    );
    let again = kept_in(&kept, "C3").unwrap();
    assert_eq!(again.fields().environment.as_deref(), Some("C3"));
    let folded = fold_back(
        &kept,
        &plan(1, PlanState::Accepted),
        RunOutcome::Succeeded,
        None,
        vec![],
        &at("09:30:00"),
    )
    .unwrap();
    assert_eq!(folded.fields().environment.as_deref(), Some("C12"));
    assert_eq!(kept_in(&folded, "C12"), Err(BoardRuleError::RunEnded(1)));
    assert_eq!(
        kept_in(&ended(&kept), "C12"),
        Err(BoardRuleError::RunEnded(1))
    );
    for bad in ["C0", "C01", "12", "c1", "C", "C1234567890", "C1 "] {
        assert!(
            matches!(
                kept_in(&open_run(1, 1), bad),
                Err(BoardRuleError::Invalid(error)) if error.field == "environment"
            ),
            "{bad}"
        );
    }
}
