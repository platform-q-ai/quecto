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

#[test]
fn the_holder_links_a_pull_request() {
    for status in [Claimed, InProgress, Review, Blocked] {
        let linked = add_pr(&task(status), &person("Ada"), 2490, &at("10:00:00")).unwrap();
        assert_eq!(linked.fields().prs, [2482, 2483, 2490], "{status:?}");
        assert_eq!(linked.fields().updated, at("10:00:00"));
    }
}

#[test]
fn only_the_holder_of_a_live_claim_links_a_new_pull_request() {
    let claimed = task(Claimed);
    assert_eq!(
        add_pr(&claimed, &person("Bob"), 2490, &at("10:00:00")),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        add_pr(&claimed, &person("Ada"), 2490, &at("12:00:00")),
        Err(BoardRuleError::ClaimExpired)
    );
    assert_eq!(
        add_pr(&claimed, &person("Ada"), 2482, &at("10:00:00")),
        Err(BoardRuleError::PrAlreadyLinked(2482))
    );
    assert!(matches!(
        add_pr(&claimed, &person("Ada"), 0, &at("10:00:00")),
        Err(BoardRuleError::Invalid(_))
    ));
    assert_eq!(
        add_pr(
            &without_claim(task(Review)),
            &person("Ada"),
            2490,
            &at("10:00:00")
        ),
        Err(BoardRuleError::NoClaim)
    );
}

#[test]
fn a_task_nobody_holds_links_no_pull_request_and_a_behind_clock_stamps_forward() {
    for status in [Ready, Done] {
        let refused = add_pr(&task(status), &person("Ada"), 2490, &at("10:00:00"));
        assert_eq!(refused, Err(BoardRuleError::NoClaim), "{status:?}");
    }
    let written = last_written(task(Claimed), &at("10:30:00"));
    let linked = add_pr(&written, &person("Ada"), 2490, &at("10:00:00")).unwrap();
    assert_eq!(
        linked.fields().updated,
        at("10:30:00"),
        "never stamped behind the task"
    );
}
