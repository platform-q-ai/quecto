//! #2481 M4c: a discard is the claim holder's decision on a draft, pinned
//! as an edit is.
use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, person, plan, task};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::Ready;

/// `now`, with no outside bound.
fn when(now: &Timestamp) -> Moment {
    Moment {
        now: now.clone(),
        latest: None,
    }
}

/// #2481 M4c: a discard is pinned as an edit is: to the draft as the
/// caller read it, and never to a draft a clock running ahead stamped.
#[test]
fn a_discard_pinned_to_an_older_read_or_a_plan_from_ahead_is_refused() {
    let draft = plan(2, PlanState::Draft);
    let bob = person("Bob");
    let now = when(&at("10:00:00"));
    assert_eq!(
        discard(&task(Ready), &draft, Some(&at("08:00:00")), &bob, &now),
        Err(BoardRuleError::PlanChanged {
            version: 2,
            updated: at("08:30:00"),
        })
    );
    let read = discard(&task(Ready), &draft, Some(&at("08:30:00")), &bob, &now).unwrap();
    assert_eq!(read.fields().state, PlanState::Discarded);
    let bounded = Moment {
        now: at("08:00:00"),
        latest: Some(at("08:00:00")),
    };
    assert_eq!(
        discard(&task(Ready), &draft, None, &bob, &bounded),
        Err(BoardRuleError::PlanAhead {
            version: 2,
            updated: at("08:30:00"),
        })
    );
}
