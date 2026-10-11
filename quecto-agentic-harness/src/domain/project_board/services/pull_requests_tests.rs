use super::*;
use crate::domain::project_board::services::fixtures_tests::{
    at, far_future, last_written, person, task, without_claim,
};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::*;

#[test]
fn the_holder_unlinks_a_pull_request_closed_without_merging() {
    for status in [Claimed, InProgress, Review, Blocked] {
        let unlinked = remove_pr(&task(status), &person("Ada"), 2483, &at("10:00:00")).unwrap();
        let fields = unlinked.fields();
        assert_eq!(
            (fields.prs.as_slice(), fields.status, &fields.updated),
            (&[2482][..], status, &at("10:00:00")),
            "{status:?}"
        );
    }
}

#[test]
fn only_the_holder_of_a_live_claim_unlinks_a_linked_pull_request() {
    let cases = [
        (task(InProgress), person("Bob"), 2483, at("10:00:00")),
        (task(InProgress), person("Ada"), 2483, at("11:00:00")),
        (
            without_claim(task(Review)),
            person("Ada"),
            2483,
            at("10:00:00"),
        ),
        (task(Done), person("Ada"), 2483, at("10:00:00")),
        (task(InProgress), person("Ada"), 7, at("10:00:00")),
    ];
    let refusals = [
        BoardRuleError::NotHolder,
        BoardRuleError::ClaimExpired,
        BoardRuleError::NoClaim,
        BoardRuleError::NoClaim,
        BoardRuleError::PrNotLinked(7),
    ];
    for ((held, actor, pr, now), refusal) in cases.into_iter().zip(refusals) {
        assert_eq!(remove_pr(&held, &actor, pr, &now), Err(refusal));
    }
}

#[test]
fn unlinking_behind_the_task_stamps_the_task_s_own_time() {
    let ahead = last_written(task(InProgress), &far_future());
    let unlinked = remove_pr(&ahead, &person("Ada"), 2483, &at("10:00:00")).unwrap();
    assert_eq!(unlinked.fields().updated, far_future());
}
