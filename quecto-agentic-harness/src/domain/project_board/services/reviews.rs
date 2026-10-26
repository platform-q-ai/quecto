//! A change's review (#2481's artifact model): exactly one per change,
//! begun by its first round. Anyone reviews (a person or an agent); each
//! round is pinned to the commit it judged and stamped when it is added.
//!
//! The verdict is the reviewer's call: a round may approve with critical
//! findings, a round may follow an approval, and two rounds may judge the
//! same commit.
//!
//! What the add-round use case (M4) must do around this rule:
//! - load the one review stored for the change (its file is named by the
//!   task and the change, so there is never a second) and pass it, or
//!   `None` when the change has none yet;
//! - refuse while the change's review file is set aside, never passing
//!   `None` for it: that would begin a second review over the unreadable
//!   one;
//! - check with the host that the judged commit is on the change's head
//!   (its repository and branch), and refuse one that is not.
use super::super::entities::change::Change;
use super::super::entities::claim::Identity;
use super::super::entities::review::{Finding, Review, ReviewFields, Round};
use super::super::value_objects::code_ref::CommitId;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::vocabulary::ReviewVerdict;
use super::claims::{after_last, within_slack};
use super::plans::Moment;

/// What a round says: the commit judged, the verdict and the findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundDraft {
    pub commit: CommitId,
    pub verdict: ReviewVerdict,
    pub findings: Vec<Finding>,
}

/// `change`'s `review` (or a new one, the change's first) with a round of
/// `draft` by `reviewer` added `when`: at its `now`, or, if later, at
/// the review's last change (so rounds stay in order, #2705 B9) or, for
/// the first round, when the change last changed (a review never predates
/// the change it judges, as its slack check reads).
///
/// # Errors
/// A change unlinked; a review of another change (`not_this_change`); for
/// a first round, a change stamped past `when`'s latest time plus the
/// stamp's slack (`change_ahead`); a review stamped past
/// `when`'s latest time plus the stamp's slack; or a review its schema
/// refuses (a finding out of shape, past its limits).
pub fn add_round(
    change: &Change,
    review: Option<&Review>,
    draft: RoundDraft,
    reviewer: &Identity,
    when: &Moment,
) -> Result<Review, BoardRuleError> {
    let now = &when.now;
    let target = change.fields();
    match change.is_linked() {
        true => {}
        false => return Err(BoardRuleError::ChangeUnlinked(target.number)),
    }
    let (mut next, at) = match review {
        Some(review) => {
            let fields = review.fields();
            match (fields.task == target.task, fields.change == target.number) {
                (true, true) => {}
                (_, _) => return Err(BoardRuleError::NotThisChange),
            }
            match within_slack(&fields.updated, when.latest.as_ref()) {
                true => {}
                false => {
                    return Err(BoardRuleError::ReviewAhead {
                        change: fields.change,
                        updated: fields.updated.clone(),
                    });
                }
            }
            (fields.clone(), after_last(&fields.updated, now))
        }
        None => {
            // The first round is stamped no earlier than the change: a
            // change a clock running ahead stamped would drag it ahead.
            match within_slack(&target.updated, when.latest.as_ref()) {
                true => {}
                false => {
                    return Err(BoardRuleError::ChangeAhead {
                        change: target.number,
                        updated: target.updated.clone(),
                    });
                }
            }
            let begun = ReviewFields {
                task: target.task.clone(),
                change: target.number,
                rounds: vec![],
                created: now.clone(),
                updated: now.clone(),
            };
            (begun, now.clone().max(target.updated.clone()))
        }
    };
    let round = Round {
        commit: draft.commit,
        reviewer: reviewer.clone(),
        at: at.clone(),
        verdict: draft.verdict,
        findings: draft.findings,
    };
    next.created = match review {
        Some(_) => next.created,
        None => at.clone(),
    };
    next.rounds.push(round.clone());
    next.updated = at;
    let added = Review::new(next).map_err(BoardRuleError::Invalid)?;
    let fields = added.fields();
    assert_eq!(
        fields.rounds.last(),
        Some(&round),
        "the last round is the one drafted"
    );
    assert_eq!(
        (&fields.task, fields.change),
        (&target.task, target.number),
        "the change's review"
    );
    assert!(
        review.is_none_or(|review| fields.updated >= review.fields().updated),
        "a round never moves updated backwards"
    );
    Ok(added)
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
