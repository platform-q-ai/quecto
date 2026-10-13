use super::*;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::fixtures_tests::{
    at, last_written, open_run, person, plan, task, without_claim,
};
use crate::domain::project_board::value_objects::vocabulary::PlanState;

fn accepted() -> Plan {
    plan(1, PlanState::Accepted)
}

fn start_at(task: &Task, runs: &[Run], by: &str, now: &str) -> Result<Started, BoardRuleError> {
    start(task, runs, Some(&accepted()), &person(by), &at(now))
}

/// A state a start may find: the task, its runs, the starter, the time,
/// and whether the start takes the claim or why it refuses.
type Row = (
    &'static str,
    Task,
    Vec<Run>,
    &'static str,
    &'static str,
    Result<bool, BoardRuleError>,
);

/// Every state a start may find, and what it does or why it refuses.
#[test]
fn a_start_claims_a_ready_task_or_works_under_the_starters_live_claim() {
    let ended = |run: Run| {
        let mut fields = run.into_fields();
        fields.ended = Some(at("09:30:00"));
        fields.outcome =
            Some(crate::domain::project_board::value_objects::vocabulary::RunOutcome::Lost);
        Run::new(fields).unwrap()
    };
    #[rustfmt::skip] // One readable state per row.
    let table: [Row; 13] = [
        ("ready", task(Ready), vec![], "Bob", "10:00:00", Ok(true)),
        ("claimed by the starter", task(Claimed), vec![], "Ada", "10:00:00", Ok(false)),
        ("in progress by the starter", task(InProgress), vec![], "Ada", "10:00:00", Ok(false)),
        ("in review by the starter", task(Review), vec![], "Ada", "10:00:00", Ok(false)),
        ("blocked by the starter", task(Blocked), vec![], "Ada", "10:00:00", Ok(false)),
        ("a run ended before", task(InProgress), vec![ended(open_run(1, 1))], "Ada", "10:00:00", Ok(false)),
        ("held by someone else", task(InProgress), vec![], "Bob", "10:00:00", Err(BoardRuleError::NotHolder)),
        ("the starter's claim expired", task(Claimed), vec![], "Ada", "11:30:00", Err(BoardRuleError::ClaimExpired)),
        ("held by nobody", without_claim(task(Review)), vec![], "Ada", "10:00:00", Err(BoardRuleError::NoClaim)),
        ("a run open", task(InProgress), vec![open_run(1, 1)], "Ada", "10:00:00", Err(BoardRuleError::RunOpen(1))),
        ("a draft", task(Draft), vec![], "Ada", "10:00:00", Err(BoardRuleError::NotAllowed { from: Draft, to: InProgress })),
        ("done", task(Done), vec![], "Ada", "10:00:00", Err(BoardRuleError::NotAllowed { from: Done, to: InProgress })),
        ("archived", task(Archived), vec![], "Ada", "10:00:00", Err(BoardRuleError::NotAllowed { from: Archived, to: InProgress })),
    ];
    for (state, task, runs, by, now, want) in table {
        let got = start_at(&task, &runs, by, now).map(|started| {
            let fields = started.task.fields();
            assert_eq!(fields.status, InProgress, "{state}");
            let claim = fields.claim.as_ref().expect("a started task is held");
            assert!(claim.holder.is(&person(by)), "{state}");
            assert!(started.run.is_open(), "{state}");
            let number = runs.len() as u32 + 1;
            assert_eq!(started.run.fields().number, number, "{state}");
            assert_eq!(started.run.fields().plan_version, 1, "{state}");
            started.took_claim
        });
        assert_eq!(got, want, "{state}");
    }
}

/// The claim, the move and the run share one stamp, as one commit writes
/// them: the caller's time, or a second past the task's last change.
#[test]
fn a_start_stamps_its_claim_its_move_and_its_run_alike() {
    for (written, now, stamped) in [
        ("09:00:00", "10:00:00", "10:00:00"),
        ("10:30:00", "10:00:00", "10:30:01"),
    ] {
        let ready = last_written(task(Ready), &at(written));
        let started = start_at(&ready, &[], "Bob", now).unwrap();
        let fields = started.task.fields();
        let claim = fields.claim.as_ref().unwrap();
        assert_eq!(fields.updated, at(stamped));
        assert_eq!(claim.since, at(stamped));
        assert_eq!(
            claim.expires,
            at(stamped).plus_seconds(2 * 60 * 60).unwrap()
        );
        assert_eq!(started.run.fields().started, at(stamped));
    }
    // A held task keeps its claim as it was.
    let held = task(InProgress);
    let started = start_at(&held, &[], "Ada", "10:00:00").unwrap();
    assert_eq!(started.task.fields().claim, held.fields().claim);
    assert_eq!(started.run.fields().started, at("10:00:00"));
}

/// Only the task's accepted plan starts: none accepted, its version
/// missing, or another one given, is refused before anything is written.
#[test]
fn a_start_runs_only_the_tasks_accepted_plan() {
    let mut fields = task(Ready).into_fields();
    fields.accepted_plan = None;
    let unplanned = Task::new(fields).unwrap();
    for given in [None, Some(accepted())] {
        assert_eq!(
            start(
                &unplanned,
                &[],
                given.as_ref(),
                &person("Bob"),
                &at("10:00:00")
            ),
            Err(BoardRuleError::NoAcceptedPlan)
        );
    }
    assert_eq!(
        start(&task(Ready), &[], None, &person("Bob"), &at("10:00:00")),
        Err(BoardRuleError::UnknownPlanVersion(1))
    );
    for (given, refused) in [
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
            start(
                &task(Ready),
                &[],
                Some(&given),
                &person("Bob"),
                &at("10:00:00")
            ),
            Err(refused)
        );
    }
}

/// A start begins from ready (taking the claim) or from a status its
/// holder may move to in progress (or is in already); every other status
/// is refused as a move to in progress.
#[test]
fn a_start_begins_only_from_ready_or_a_held_status() {
    let starts: Vec<TaskStatus> = TaskStatus::ALL
        .iter()
        .copied()
        .filter(|status| start_at(&task(*status), &[], "Ada", "10:00:00").is_ok())
        .collect();
    assert_eq!(starts, [Ready, Claimed, InProgress, Review, Blocked]);
}
