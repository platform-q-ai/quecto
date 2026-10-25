//! #2683: a release is a holder-only action like any other: another's
//! expired claim is refused with the take_over way on.
use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, person, task, without_prs};

/// #2683: releasing another's expired claim (claimed back to ready; a
/// task in progress goes back by its run's end, #2705 T5) is refused with
/// the take_over way on, as every holder-only action is; a live one is
/// plainly not the mover's, and the holder still releases its own expired
/// claim.
#[test]
fn another_releasing_an_expired_claim_is_told_to_take_it_over() {
    let bob = person("Bob");
    let held = without_prs(task(Claimed));
    assert_eq!(
        move_task(&held, Ready, &bob, (&[], &[]), &[], &at("14:00:00")),
        Err(BoardRuleError::NotHolderExpired)
    );
    assert_eq!(
        move_task(&held, Ready, &bob, (&[], &[]), &[], &at("10:00:00")),
        Err(BoardRuleError::NotHolder)
    );
    let ada = person("Ada");
    assert!(move_task(&held, Ready, &ada, (&[], &[]), &[], &at("14:00:00")).is_ok());
}
