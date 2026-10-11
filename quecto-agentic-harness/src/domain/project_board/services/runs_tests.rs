use super::*;
use crate::domain::project_board::entities::task_parts::Run;
use crate::domain::project_board::services::claims::take_over;
use crate::domain::project_board::services::fixtures_tests::{
    at, far_future, last_written, person, task, with_open_run,
};
use crate::domain::project_board::services::transitions::move_task;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{RunOutcome, TaskStatus::*};

fn the_run(task: &Task) -> &Run {
    let id = Slug::parse("r2").unwrap();
    task.fields().runs.iter().find(|run| run.id == id).unwrap()
}

#[test]
fn the_holder_ends_an_open_run_as_lost_and_can_then_hand_the_task_back() {
    for status in [Claimed, InProgress, Review, Blocked] {
        let ended = end_run_lost(
            &with_open_run(task(status)),
            &person("Ada"),
            &at("10:00:00"),
        )
        .unwrap();
        let run = the_run(&ended);
        assert_eq!(
            (&run.ended, run.outcome, &ended.fields().updated),
            (
                &Some(at("10:00:00")),
                Some(RunOutcome::Lost),
                &at("10:00:00")
            ),
            "{status:?}"
        );
        assert!(move_task(&ended, Ready, &person("Ada"), &at("10:00:00")).is_ok());
    }
}

#[test]
fn whoever_takes_over_an_expired_claim_ends_its_run() {
    let stalled = with_open_run(task(Review));
    let taken = take_over(&stalled, &person("Bob"), &at("11:30:00")).unwrap();
    let ended = end_run_lost(&taken, &person("Bob"), &at("11:31:00")).unwrap();
    assert_eq!(the_run(&ended).outcome, Some(RunOutcome::Lost));
}

#[test]
fn only_the_holder_of_a_live_claim_ends_an_open_run() {
    let running = with_open_run(task(InProgress));
    assert_eq!(
        end_run_lost(&running, &person("Bob"), &at("10:00:00")),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        end_run_lost(&running, &person("Ada"), &at("11:00:00")),
        Err(BoardRuleError::ClaimExpired)
    );
    assert_eq!(
        end_run_lost(&task(InProgress), &person("Ada"), &at("10:00:00")),
        Err(BoardRuleError::NoOpenRun)
    );
}

#[test]
fn a_run_ended_behind_the_task_ends_at_the_task_s_own_time() {
    let ahead = last_written(with_open_run(task(InProgress)), &far_future());
    let ended = end_run_lost(&ahead, &person("Ada"), &at("10:00:00")).unwrap();
    assert_eq!(
        the_run(&ended).ended,
        Some(far_future().plus_seconds(1).unwrap())
    );
}
