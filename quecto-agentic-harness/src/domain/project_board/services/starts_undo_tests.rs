//! #2705 T4: a start's undo takes an in-progress task back to the review
//! or blocked status it was started from, now that `move` no longer does:
//! the holder's, once its run has ended, keeping the claim.
use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, open_run, person, task};

#[test]
fn an_undone_start_goes_back_to_review_or_blocked_keeping_the_claim() {
    for from in [Review, Blocked] {
        let back = unstart(
            &task(InProgress),
            from,
            &[],
            &person("Ada"),
            &at("10:00:00"),
        )
        .unwrap();
        let fields = back.fields();
        assert_eq!(
            (fields.status, &fields.claim, &fields.updated),
            (from, &task(InProgress).fields().claim, &at("10:00:00")),
            "{from:?}"
        );
    }
}

#[test]
fn an_undone_start_waits_for_its_run_and_is_the_holders() {
    let going = [open_run(1, 1)];
    assert_eq!(
        unstart(
            &task(InProgress),
            Review,
            &going,
            &person("Ada"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::RunOpen(1))
    );
    assert_eq!(
        unstart(
            &task(InProgress),
            Blocked,
            &[],
            &person("Bob"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::NotHolder)
    );
}

#[test]
fn only_an_in_progress_task_goes_back_and_only_to_review_or_blocked() {
    let ada = person("Ada");
    let now = at("10:00:00");
    for to in [Draft, Ready, Claimed, InProgress, Done, Archived] {
        assert_eq!(
            unstart(&task(InProgress), to, &[], &ada, &now),
            Err(BoardRuleError::NotAllowed {
                from: InProgress,
                to
            }),
            "to {to:?}"
        );
    }
    for from in [Claimed, Review, Blocked] {
        assert_eq!(
            unstart(&task(from), Review, &[], &ada, &now),
            Err(BoardRuleError::NotAllowed { from, to: Review }),
            "from {from:?}"
        );
    }
}
