//! #2705 T5: a run's end moves its task; item states and inconsistencies
//! are read from the task's runs.
use super::*;
use crate::domain::project_board::entities::run::{RunFields, RunItem};
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::services::fixtures_tests::{
    at, open_run, plan, task, without_claim,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, Resolution};

/// Run `number` of plan `version`, ended at 10:00 as `outcome`, its items
/// standing as `items` says.
fn ended(number: u32, version: u32, outcome: RunOutcome, items: &[(&str, ItemState)]) -> Run {
    Run::new(RunFields {
        ended: Some(at("10:00:00")),
        outcome: Some(outcome),
        items: items
            .iter()
            .map(|(id, state)| RunItem {
                id: Slug::parse(id).unwrap(),
                state: *state,
                owner: None,
                evidence: vec![],
            })
            .collect(),
        ..open_run(number, version).into_fields()
    })
    .unwrap()
}

fn with(task: Task, change: impl FnOnce(&mut TaskFields)) -> Task {
    let mut fields = task.into_fields();
    change(&mut fields);
    Task::new(fields).unwrap()
}

#[test]
fn a_run_end_moves_its_task_by_its_outcome() {
    use RunOutcome::*;
    for (outcome, stop, want) in [
        (Succeeded, StopKind::Cancelled, Review),
        (Succeeded, StopKind::Blocked, Review),
        (Stopped, StopKind::Blocked, Blocked),
        (Stopped, StopKind::Cancelled, Ready),
        (Failed, StopKind::Blocked, Ready),
        (Lost, StopKind::Cancelled, Ready),
    ] {
        assert_eq!(status_after(outcome, stop), want, "{outcome:?} {stop:?}");
    }
}

#[test]
fn a_success_moves_a_task_in_progress_to_review_keeping_its_claim() {
    let held = task(InProgress);
    let moved = task_after_end(
        &held,
        &ended(1, 1, RunOutcome::Succeeded, &[]),
        StopKind::Cancelled,
        &at("10:00:00"),
    )
    .unwrap()
    .expect("the task moves");
    let fields = moved.fields();
    assert_eq!(fields.status, Review);
    assert_eq!(fields.claim, held.fields().claim, "its holder keeps it");
    assert_eq!(fields.updated, at("10:00:00"));
    assert_eq!(fields.reason, None);
}

#[test]
fn a_blocked_stop_moves_it_to_blocked_keeping_its_claim() {
    let held = task(InProgress);
    let moved = task_after_end(
        &held,
        &ended(1, 1, RunOutcome::Stopped, &[]),
        StopKind::Blocked,
        &at("10:00:00"),
    )
    .unwrap()
    .expect("the task moves");
    assert_eq!(moved.fields().status, Blocked);
    assert_eq!(moved.fields().claim, held.fields().claim);
}

#[test]
fn any_other_end_moves_it_back_to_ready_releasing_its_claim() {
    for (outcome, stop) in [
        (RunOutcome::Stopped, StopKind::Cancelled),
        (RunOutcome::Failed, StopKind::Cancelled),
        (RunOutcome::Lost, StopKind::Blocked),
    ] {
        for from in [Claimed, InProgress, Review, Blocked] {
            let moved = task_after_end(
                &task(from),
                &ended(1, 1, outcome, &[]),
                stop,
                &at("10:00:00"),
            )
            .unwrap()
            .expect("the task moves");
            let fields = moved.fields();
            assert_eq!(fields.status, Ready, "{from:?} after {outcome:?}");
            assert_eq!(fields.claim, None, "{from:?}: released");
            assert_eq!(fields.updated, at("10:00:00"));
        }
    }
}

#[test]
fn a_task_already_where_the_end_moves_it_or_not_held_is_left() {
    let review = task(Review);
    assert_eq!(
        task_after_end(
            &review,
            &ended(1, 1, RunOutcome::Succeeded, &[]),
            StopKind::Cancelled,
            &at("10:00:00"),
        ),
        Ok(None),
        "a task in review stays as it is"
    );
    for status in [Draft, Ready, Done, Archived] {
        let unheld = with(task(status), |fields| fields.prs.clear());
        assert_eq!(
            task_after_end(
                &unheld,
                &ended(1, 1, RunOutcome::Failed, &[]),
                StopKind::Cancelled,
                &at("10:00:00"),
            ),
            Ok(None),
            "{status:?}: a legacy task nobody holds is left"
        );
    }
}

#[test]
fn a_run_still_going_or_of_another_task_moves_nothing() {
    assert_eq!(
        task_after_end(
            &task(InProgress),
            &open_run(1, 1),
            StopKind::Cancelled,
            &at("10:00:00"),
        ),
        Err(BoardRuleError::RunOpen(1))
    );
    let other = with(task(InProgress), |fields| {
        fields.id = Slug::parse("other-task").unwrap();
    });
    assert_eq!(
        task_after_end(
            &other,
            &ended(1, 1, RunOutcome::Failed, &[]),
            StopKind::Cancelled,
            &at("10:00:00"),
        ),
        Err(BoardRuleError::NotThisTasks)
    );
}

#[test]
fn items_with_no_run_read_not_started() {
    let accepted = plan(1, PlanState::Accepted);
    assert_eq!(
        item_standings(&accepted, None),
        [ItemStanding::NotStarted; 2]
    );
    assert_eq!(ItemStanding::NotStarted.as_str(), "not_started");
}

#[test]
fn items_read_their_state_from_the_latest_run_of_the_plan() {
    let accepted = plan(1, PlanState::Accepted);
    let latest = ended(
        2,
        1,
        RunOutcome::Failed,
        &[("schema", ItemState::Done), ("api", ItemState::Blocked)],
    );
    assert_eq!(
        item_standings(&accepted, Some(&latest)),
        [
            ItemStanding::In(ItemState::Done),
            ItemStanding::In(ItemState::Blocked)
        ]
    );
    assert_eq!(
        item_standings(&accepted, Some(&open_run(3, 1))),
        [ItemStanding::In(ItemState::Todo); 2],
        "a run going records its items when it ends"
    );
    assert_eq!(
        item_standings(&plan(2, PlanState::Accepted), Some(&latest)),
        [ItemStanding::NotStarted; 2],
        "a plan the latest run did not run has not started"
    );
}

/// The land-layout-2652 case: a task done by a swarm spawned by hand,
/// with no plan accepted and no run.
#[test]
fn a_done_task_with_no_plan_and_no_run_reads_done_without_plan() {
    let done = with(task(Done), |fields| fields.accepted_plan = None);
    assert_eq!(
        inconsistencies(&done, &[]),
        [Inconsistency::DoneWithoutPlan]
    );
    assert_eq!(Inconsistency::DoneWithoutPlan.as_str(), "done_without_plan");
    let without_run = with(done.clone(), |fields| {
        fields.resolution = Some(Resolution::DoneWithoutRun);
        fields.reason = Some("Fixed by hand in #2652".into());
    });
    assert_eq!(
        inconsistencies(&without_run, &[]),
        [],
        "done without a run says so"
    );
}

#[test]
fn a_task_that_contradicts_its_runs_reads_so_and_a_started_one_does_not() {
    assert_eq!(inconsistencies(&task(InProgress), &[open_run(1, 1)]), []);
    assert_eq!(
        inconsistencies(&task(InProgress), &[]),
        [Inconsistency::InProgressWithoutRun]
    );
    assert_eq!(
        inconsistencies(&task(InProgress), &[ended(1, 1, RunOutcome::Stopped, &[])]),
        [Inconsistency::InProgressWithoutRun]
    );
    assert_eq!(
        inconsistencies(&without_claim(task(Review)), &[open_run(1, 1)]),
        [Inconsistency::RunGoingOutsideProgress]
    );
    assert_eq!(
        inconsistencies(&task(Review), &[ended(1, 1, RunOutcome::Succeeded, &[])]),
        []
    );
}

/// #2705 T5 review round 1: a start's undo puts a task it took from
/// review or blocked back there, claim kept; any other goes as a failed
/// run's end takes it, back to ready, released.
#[test]
fn a_starts_undo_restores_review_or_blocked_and_else_releases() {
    let held = task(InProgress);
    let failed = ended(1, 1, RunOutcome::Failed, &[]);
    for from in [Review, Blocked] {
        let restored = task_after_undo(&held, &failed, from, &at("10:00:00"))
            .unwrap()
            .expect("the task moves back");
        assert_eq!(restored.fields().status, from);
        assert_eq!(restored.fields().claim, held.fields().claim, "{from:?}");
    }
    for from in [Ready, Claimed, InProgress] {
        let released = task_after_undo(&held, &failed, from, &at("10:00:00"))
            .unwrap()
            .expect("the task moves");
        let fields = released.fields();
        assert_eq!(
            (fields.status, fields.claim.clone()),
            (Ready, None),
            "{from:?}"
        );
    }
    assert_eq!(
        task_after_undo(&held, &open_run(1, 1), Review, &at("10:00:00")),
        Err(BoardRuleError::RunOpen(1))
    );
}

#[test]
fn a_stop_kind_is_named_by_its_allowlist() {
    for stop in StopKind::ALL {
        assert_eq!(StopKind::parse(stop.as_str()), Some(stop));
    }
    assert_eq!(StopKind::parse("paused"), None);
}
