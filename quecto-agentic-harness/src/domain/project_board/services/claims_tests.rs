use super::*;
use crate::domain::project_board::services::fixtures_tests::{
    at, far_future, last_written, person, task, without_claim,
};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::*;

#[test]
fn the_holder_renews_for_two_hours_from_now() {
    let renewed = renew(&task(InProgress), &person("Ada"), &at("10:30:00")).unwrap();
    let fields = renewed.fields();
    let claim = fields.claim.clone().unwrap();
    assert_eq!(
        (claim.holder, claim.since, claim.expires),
        (person("Ada"), at("10:30:00"), at("12:30:00"))
    );
    assert_eq!(
        (fields.status, &fields.updated),
        (InProgress, &at("10:30:00"))
    );
    assert_eq!(CLAIM_TTL_SECONDS, 2 * 60 * 60);
}

#[test]
fn an_expired_claim_is_still_renewed_by_its_holder_until_taken() {
    let renewed = renew(&task(Claimed), &person("Ada"), &at("13:00:00")).unwrap();
    assert_eq!(
        renewed.fields().claim.as_ref().map(|claim| &claim.expires),
        Some(&at("15:00:00"))
    );
}

#[test]
fn only_a_held_task_is_renewed_and_only_by_its_holder() {
    assert_eq!(
        renew(&task(Claimed), &person("Bob"), &at("10:30:00")),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        renew(&task(Ready), &person("Ada"), &at("10:30:00")),
        Err(BoardRuleError::NoClaim)
    );
}

#[test]
fn an_expired_claim_is_taken_over_while_the_task_may_hold_one() {
    for status in [Claimed, InProgress, Review, Blocked] {
        let taken = take_over(&task(status), &person("Bob"), &at("11:00:00")).unwrap();
        let fields = taken.fields();
        let claim = fields.claim.clone().unwrap();
        assert_eq!(
            (claim.holder, claim.since, claim.expires),
            (person("Bob"), at("11:00:00"), at("13:00:00")),
            "{status:?}"
        );
        assert_eq!((fields.status, &fields.updated), (status, &at("11:00:00")));
    }
}

#[test]
fn a_live_claim_another_status_no_claim_or_the_holder_cannot_take_over() {
    let bob = person("Bob");
    for status in [Claimed, InProgress, Review, Blocked] {
        assert_eq!(
            take_over(&task(status), &bob, &at("10:59:59")),
            Err(BoardRuleError::ClaimLive),
            "{status:?}"
        );
    }
    for status in [Draft, Ready, Done, Archived] {
        assert_eq!(
            take_over(&task(status), &bob, &at("12:00:00")),
            Err(BoardRuleError::NotTakeable(status))
        );
    }
    for status in [Review, Blocked] {
        assert_eq!(
            take_over(&without_claim(task(status)), &bob, &at("12:00:00")),
            Err(BoardRuleError::NoClaim)
        );
    }
    let shouting = Identity {
        name: "ADA".into(),
        email: "ADA@example.com".into(),
    };
    assert_eq!(
        take_over(&task(Claimed), &shouting, &at("12:00:00")),
        Err(BoardRuleError::AlreadyHolder)
    );
}

#[test]
fn a_renewal_by_another_name_keeps_the_holder_as_claimed() {
    let renamed = Identity {
        name: "A. Lovelace".into(),
        email: "ada@EXAMPLE.com".into(),
    };
    let renewed = renew(&task(InProgress), &renamed, &at("10:30:00")).unwrap();
    assert_eq!(
        renewed.fields().claim.as_ref().map(|claim| &claim.holder),
        Some(&person("Ada"))
    );
}

#[test]
fn renewing_or_taking_over_behind_the_task_stamps_the_task_s_own_time() {
    let ahead = last_written(task(InProgress), &far_future());
    let next = far_future().plus_seconds(1).unwrap();
    let in_two_hours = next.plus_seconds(CLAIM_TTL_SECONDS).unwrap();
    for changed in [
        renew(&ahead, &person("Ada"), &at("10:30:00")).unwrap(),
        take_over(&ahead, &person("Bob"), &at("11:00:00")).unwrap(),
    ] {
        let claim = changed.fields().claim.clone().unwrap();
        assert_eq!(
            (&claim.since, &claim.expires, &changed.fields().updated),
            (&next, &in_two_hours, &next)
        );
    }
}

/// Two changes in one second, or one from a clock behind the task, never
/// share a stamp: `updated` moves strictly forward on every change.
#[test]
fn every_change_moves_updated_strictly_forward() {
    let renewed = renew(&task(InProgress), &person("Ada"), &at("10:00:00")).unwrap();
    let again = renew(&renewed, &person("Ada"), &at("10:00:00")).unwrap();
    assert_eq!(
        (&renewed.fields().updated, &again.fields().updated),
        (&at("10:00:00"), &at("10:00:01"))
    );
    let behind = renew(&again, &person("Ada"), &at("09:30:00")).unwrap();
    assert_eq!(behind.fields().updated, at("10:00:02"));
}
