use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, person, task};

const ALL: [TaskStatus; 8] = [
    Draft, Ready, Claimed, InProgress, Review, Blocked, Done, Archived,
];
/// Written out independently of the production table.
const ALLOWED: [(TaskStatus, TaskStatus); 11] = [
    (Draft, Ready),
    (Blocked, Ready),
    (Ready, Claimed),
    (Claimed, InProgress),
    (InProgress, Review),
    (InProgress, Blocked),
    (Review, Done),
    (Review, InProgress),
    (Done, Archived),
    (Claimed, Ready),
    (InProgress, Ready),
];

#[test]
fn only_the_table_allows_a_move_even_for_the_holder() {
    for from in ALL {
        for to in ALL {
            let moved = move_task(&task(from), to, &person("Ada"), &at("10:00:00"));
            match moved {
                Ok(next) => {
                    assert!(
                        ALLOWED.contains(&(from, to)),
                        "{from:?} -> {to:?} must be refused"
                    );
                    assert_eq!(
                        (next.fields().status, &next.fields().updated),
                        (to, &at("10:00:00"))
                    );
                }
                Err(error) => {
                    assert!(
                        !ALLOWED.contains(&(from, to)),
                        "{from:?} -> {to:?}: {error}"
                    );
                    assert_eq!(error, BoardRuleError::NotAllowed { from, to });
                }
            }
        }
    }
}

#[test]
fn anyone_readies_claims_and_archives_but_only_the_holder_moves_a_held_task() {
    let bob = person("Bob");
    for (from, to) in ALLOWED {
        let moved = move_task(&task(from), to, &bob, &at("10:00:00"));
        let anyones = matches!(
            (from, to),
            (Draft, Ready) | (Blocked, Ready) | (Ready, Claimed) | (Done, Archived)
        );
        assert_eq!(
            moved.is_ok(),
            anyones,
            "{from:?} -> {to:?} by Bob: {moved:?}"
        );
        if !anyones {
            assert_eq!(moved, Err(BoardRuleError::NotHolder));
        }
    }
}

#[test]
fn the_holder_is_known_by_email_whatever_its_case() {
    let shouting = Identity {
        name: "ADA".into(),
        email: "ADA@EXAMPLE.COM".into(),
    };
    assert!(move_task(&task(Claimed), InProgress, &shouting, &at("10:00:00")).is_ok());
}

#[test]
fn a_claim_is_taken_kept_or_cleared_by_the_move() {
    let now = at("10:00:00");
    let claimed = move_task(&task(Ready), Claimed, &person("Bob"), &now).unwrap();
    let claim = claimed.fields().claim.clone().unwrap();
    assert_eq!(
        (claim.holder, claim.since, claim.expires),
        (person("Bob"), now.clone(), at("12:00:00"))
    );
    let started = move_task(&task(Claimed), InProgress, &person("Ada"), &now).unwrap();
    assert_eq!(
        started.fields().claim,
        task(Claimed).fields().claim,
        "the holder keeps the claim"
    );
    for (from, to) in [
        (Claimed, Ready),
        (InProgress, Ready),
        (Review, Done),
        (Blocked, Ready),
    ] {
        let next = move_task(&task(from), to, &person("Ada"), &now).unwrap();
        assert_eq!(
            next.fields().claim,
            None,
            "{from:?} -> {to:?} releases the claim"
        );
    }
}

#[test]
fn a_holder_move_needs_a_claim() {
    let mut review = task(Review).into_fields();
    review.claim = None;
    let review = Task::new(review).unwrap();
    assert_eq!(
        move_task(&review, Done, &person("Ada"), &at("10:00:00")),
        Err(BoardRuleError::NoClaim)
    );
}

#[test]
fn a_task_in_review_is_done_once_all_its_prs_merge_whoever_merged_them() {
    let merged = |prs: &[u64]| prs.iter().copied().collect::<BTreeSet<u64>>();
    let done = complete_merged(&task(Review), &merged(&[2482, 2483, 9]), &at("10:00:00")).unwrap();
    let fields = done.fields();
    assert_eq!(
        (fields.status, &fields.claim, &fields.updated),
        (Done, &None, &at("10:00:00"))
    );
    let partly = complete_merged(&task(Review), &merged(&[2482]), &at("10:00:00"));
    assert_eq!(partly, Err(BoardRuleError::PrsOpen(vec![2483])));
    let refused = complete_merged(&task(InProgress), &merged(&[2482, 2483]), &at("10:00:00"));
    assert_eq!(
        refused,
        Err(BoardRuleError::NotAllowed {
            from: InProgress,
            to: Done
        })
    );
    let mut unlinked = task(Review).into_fields();
    unlinked.prs.clear();
    let unlinked = Task::new(unlinked).unwrap();
    assert_eq!(
        complete_merged(&unlinked, &merged(&[2482]), &at("10:00:00")),
        Err(BoardRuleError::NoPrs)
    );
}

#[test]
fn every_moved_task_is_validated_again() {
    let moved = move_task(&task(Draft), Ready, &person("Bob"), &at("07:00:00"));
    assert!(matches!(moved, Err(BoardRuleError::Invalid(error)) if error.field == "updated"));
}
