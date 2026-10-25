//! #2683: a release is a holder-only action like any other: another's
//! expired claim is refused with the take_over way on.
use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, person, task, without_prs};

/// #2683: releasing another's expired claim (claimed or in progress back
/// to ready) is refused with the take_over way on, as every holder-only
/// action is; a live one is plainly not the mover's, and the holder still
/// releases its own expired claim.
#[test]
fn another_releasing_an_expired_claim_is_told_to_take_it_over() {
    let bob = person("Bob");
    for from in [Claimed, InProgress] {
        let held = without_prs(task(from));
        assert_eq!(
            move_task(&held, Ready, &bob, (&[], &[]), &[], &at("14:00:00")),
            Err(BoardRuleError::NotHolderExpired),
            "{from:?}"
        );
        assert_eq!(
            move_task(&held, Ready, &bob, (&[], &[]), &[], &at("10:00:00")),
            Err(BoardRuleError::NotHolder),
            "{from:?}"
        );
        assert!(
            move_task(
                &held,
                Ready,
                &person("Ada"),
                (&[], &[]),
                &[],
                &at("14:00:00")
            )
            .is_ok(),
            "{from:?}"
        );
    }
}
