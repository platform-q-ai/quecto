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

/// A task stamped at the last moment the board can spell takes no further
/// change: its next stamp cannot be spelt.
#[test]
fn a_task_at_the_last_spellable_second_takes_no_further_stamp() {
    let last = Timestamp::parse("9999-12-31T23:59:59Z").unwrap();
    let mut fields = task(Draft).into_fields();
    fields.updated = last.clone();
    let at_the_end = Task::new(fields).unwrap();
    let moved =
        super::super::transitions::move_task(&at_the_end, Ready, &person("Ada"), &[], &last);
    assert!(
        matches!(moved, Err(BoardRuleError::Invalid(_))),
        "{moved:?}"
    );
}

/// The bound is the latest time plus the slack; where adding the slack
/// overflows what the board can spell, it tightens to the latest time.
#[test]
fn the_slack_bound_tightens_to_the_latest_time_where_it_would_overflow() {
    let time = |text: &str| Timestamp::parse(text).unwrap();
    let latest = time("2026-10-10T10:00:00Z");
    assert!(within_slack(&time("2026-10-10T10:00:30Z"), Some(&latest)));
    assert!(!within_slack(&time("2026-10-10T10:00:31Z"), Some(&latest)));
    assert!(
        within_slack(&time("9999-12-31T23:59:59Z"), None),
        "no reference"
    );
    let last = time("9999-12-31T23:59:50Z");
    assert!(within_slack(&last, Some(&last)));
    assert!(!within_slack(&time("9999-12-31T23:59:51Z"), Some(&last)));
}
