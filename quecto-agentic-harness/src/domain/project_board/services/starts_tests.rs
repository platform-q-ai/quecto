use super::*;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::fixtures_tests::{
    at, last_written, open_run, person, plan, task, without_claim,
};
use crate::domain::project_board::value_objects::vocabulary::{PlanState, RunOutcome};

fn accepted() -> Plan {
    plan(1, PlanState::Accepted)
}

fn start_at(task: &Task, runs: &[Run], by: &str, now: &str) -> Result<Started, BoardRuleError> {
    start(task, &[], runs, Some(&accepted()), &person(by), &at(now))
}

/// A state a start may find: the task, its runs, the starter, the time,
/// and the status it started from or why it refuses.
type Row = (
    &'static str,
    Task,
    Vec<Run>,
    &'static str,
    &'static str,
    Result<TaskStatus, BoardRuleError>,
);

/// Every state a start may find, and what it does or why it refuses. Ada
/// holds every held fixture task from 09:00 to 11:00.
#[test]
fn a_start_claims_a_ready_task_or_works_under_the_starters_own_claim() {
    let ended = |run: Run| {
        let mut fields = run.into_fields();
        fields.ended = Some(at("09:30:00"));
        fields.outcome = Some(RunOutcome::Lost);
        Run::new(fields).unwrap()
    };
    let not_allowed = |from| {
        Err(BoardRuleError::NotAllowed {
            from,
            to: InProgress,
        })
    };
    #[rustfmt::skip] // One readable state per row.
    let table: [Row; 23] = [
        ("ready", task(Ready), vec![], "Bob", "10:00:00", Ok(Ready)),
        ("claimed by the starter", task(Claimed), vec![], "Ada", "10:00:00", Ok(Claimed)),
        ("in progress by the starter", task(InProgress), vec![], "Ada", "10:00:00", Ok(InProgress)),
        ("in review by the starter", task(Review), vec![], "Ada", "10:00:00", Ok(Review)),
        ("blocked by the starter", task(Blocked), vec![], "Ada", "10:00:00", Ok(Blocked)),
        ("a run ended before", task(InProgress), vec![ended(open_run(1, 1))], "Ada", "10:00:00", Ok(InProgress)),
        // The holder renews an expired claim of their own, as `renew` does.
        ("claimed, the starter's claim expired", task(Claimed), vec![], "Ada", "11:30:00", Ok(Claimed)),
        ("in progress, the starter's claim expired", task(InProgress), vec![], "Ada", "11:30:00", Ok(InProgress)),
        ("in review, the starter's claim expired", task(Review), vec![], "Ada", "11:30:00", Ok(Review)),
        ("blocked, the starter's claim expired", task(Blocked), vec![], "Ada", "11:30:00", Ok(Blocked)),
        // Someone else's claim, live or expired, is theirs until taken over;
        // an expired one says so (#2683).
        ("claimed by someone else", task(Claimed), vec![], "Bob", "10:00:00", Err(BoardRuleError::NotHolder)),
        ("in progress by someone else", task(InProgress), vec![], "Bob", "10:00:00", Err(BoardRuleError::NotHolder)),
        ("in review by someone else", task(Review), vec![], "Bob", "10:00:00", Err(BoardRuleError::NotHolder)),
        ("blocked by someone else", task(Blocked), vec![], "Bob", "10:00:00", Err(BoardRuleError::NotHolder)),
        ("claimed by someone else, expired", task(Claimed), vec![], "Bob", "11:30:00", Err(BoardRuleError::NotHolderExpired)),
        ("in progress by someone else, expired", task(InProgress), vec![], "Bob", "11:30:00", Err(BoardRuleError::NotHolderExpired)),
        ("in review by someone else, expired", task(Review), vec![], "Bob", "11:30:00", Err(BoardRuleError::NotHolderExpired)),
        ("blocked by someone else, expired", task(Blocked), vec![], "Bob", "11:30:00", Err(BoardRuleError::NotHolderExpired)),
        ("held by nobody", without_claim(task(Review)), vec![], "Ada", "10:00:00", Err(BoardRuleError::NoClaim)),
        ("a run open", task(InProgress), vec![open_run(1, 1)], "Ada", "10:00:00", Err(BoardRuleError::RunOpen(1))),
        ("a draft", task(Draft), vec![], "Ada", "10:00:00", not_allowed(Draft)),
        ("done", task(Done), vec![], "Ada", "10:00:00", not_allowed(Done)),
        ("archived", task(Archived), vec![], "Ada", "10:00:00", not_allowed(Archived)),
    ];
    for (state, task, runs, by, now, want) in table {
        let got = start_at(&task, &runs, by, now).map(|started| {
            let fields = started.task().fields();
            assert_eq!(fields.status, InProgress, "{state}");
            let claim = fields.claim.as_ref().expect("a started task is held");
            assert!(claim.holder.is(&person(by)), "{state}");
            assert!(started.run().is_open(), "{state}");
            let number = runs.len() as u32 + 1;
            assert_eq!(started.run().fields().number, number, "{state}");
            assert_eq!(started.plan(), &accepted(), "{state}");
            assert_eq!(started.took_claim(), started.from() == Ready, "{state}");
            started.from()
        });
        assert_eq!(got, want, "{state}");
    }
}

/// The claim, the move and the run share one stamp, as one commit writes
/// them: the caller's time, or the task's last change if later. A
/// held claim is renewed at that stamp too, so a run never begins with
/// only seconds of its claim left.
#[test]
fn a_start_stamps_its_claim_its_move_and_its_run_alike() {
    for (status, by) in [(Ready, "Bob"), (Claimed, "Ada"), (InProgress, "Ada")] {
        for (written, now, stamped) in [
            ("09:00:00", "10:59:59", "10:59:59"),
            ("10:30:00", "10:00:00", "10:30:00"),
        ] {
            let before = last_written(task(status), &at(written));
            let started = start_at(&before, &[], by, now).unwrap();
            let fields = started.task().fields();
            let claim = fields.claim.as_ref().unwrap();
            assert_eq!(fields.updated, at(stamped), "{status:?}");
            assert_eq!(claim.since, at(stamped), "{status:?}");
            let expires = at(stamped).plus_seconds(2 * 60 * 60).unwrap();
            assert_eq!(claim.expires, expires, "{status:?}");
            assert_eq!(started.run().fields().started, at(stamped), "{status:?}");
        }
    }
}

/// Liveness is judged at the start's stamp, on the renewed claim: the
/// starter's own claim, live at the caller's time but lapsed by the stamp
/// (the task was last written a moment after it ends), still starts.
#[test]
fn liveness_is_judged_at_the_stamp_on_the_renewed_claim() {
    let lapsing = last_written(task(Claimed), &at("11:00:05"));
    let started = start_at(&lapsing, &[], "Ada", "10:30:00").unwrap();
    assert_eq!(started.run().fields().started, at("11:00:05"));
    assert_eq!(
        started.task().fields().claim.as_ref().unwrap().since,
        at("11:00:05")
    );
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
                &[],
                given.as_ref(),
                &person("Bob"),
                &at("10:00:00")
            ),
            Err(BoardRuleError::NoAcceptedPlan)
        );
    }
    assert_eq!(
        start(
            &task(Ready),
            &[],
            &[],
            None,
            &person("Bob"),
            &at("10:00:00")
        ),
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

/// A held start moves the task to in progress as the lifecycle says
/// (#2705 T4): each held status a start begins from is a start's in the
/// lifecycle, `move` refuses it naming start, and only the holder starts.
#[test]
fn every_held_start_is_the_lifecycles_and_the_holders() {
    for status in [Claimed, Review, Blocked] {
        let held = task(status);
        assert_eq!(
            super::super::transitions::lifecycle_action(status, InProgress),
            Some(crate::domain::project_board::value_objects::vocabulary::LifecycleAction::Start),
            "{status:?}"
        );
        assert!(
            matches!(
                move_reads(&held, InProgress, &person("Ada"), &at("10:00:00")),
                Err(BoardRuleError::UseLifecycle { .. })
            ),
            "{status:?}: move never makes it"
        );
        assert!(
            start_at(&held, &[], "Bob", "10:00:00").is_err(),
            "{status:?}: only the holder starts it"
        );
    }
}
