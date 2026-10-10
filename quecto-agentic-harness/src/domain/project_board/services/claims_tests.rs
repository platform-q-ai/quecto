use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, person, task};
use TaskStatus::*;

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
fn an_expired_claim_is_taken_over_while_claimed_or_in_progress() {
    for status in [Claimed, InProgress] {
        let taken = take_over(&task(status), &person("Bob"), &at("11:00:00")).unwrap();
        let fields = taken.fields();
        let claim = fields.claim.clone().unwrap();
        assert_eq!(
            (claim.holder, claim.since, claim.expires),
            (person("Bob"), at("11:00:00"), at("13:00:00"))
        );
        assert_eq!((fields.status, &fields.updated), (status, &at("11:00:00")));
    }
}

#[test]
fn a_live_claim_another_status_or_the_holder_cannot_take_over() {
    let bob = person("Bob");
    assert_eq!(
        take_over(&task(Claimed), &bob, &at("10:59:59")),
        Err(BoardRuleError::ClaimLive)
    );
    for status in [Draft, Ready, Review, Blocked, Done, Archived] {
        assert_eq!(
            take_over(&task(status), &bob, &at("12:00:00")),
            Err(BoardRuleError::NotTakeable(status))
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
