//! A change's review (#2481's artifact model): exactly one per change,
//! begun by its first round. Anyone reviews (a person or an agent); each
//! round is pinned to the commit it judged and stamped when it is added.
//!
//! What the add-round use case (M4) must do around this rule:
//! - load the one review stored for the change (its file is named by the
//!   task and the change, so there is never a second) and pass it, or
//!   `None` when the change has none yet;
//! - check with the host that the judged commit is on the change's head
//!   (its repository and branch), and refuse one that is not.
use super::super::entities::change::Change;
use super::super::entities::claim::Identity;
use super::super::entities::review::{Finding, Review, ReviewFields, Round};
use super::super::value_objects::code_ref::CommitId;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::vocabulary::ReviewVerdict;
use super::claims::within_slack;
use super::plans::Moment;

/// What a round says: the commit judged, the verdict and the findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundDraft {
    pub commit: CommitId,
    pub verdict: ReviewVerdict,
    pub findings: Vec<Finding>,
}

/// `change`'s `review` (or a new one, the change's first) with a round of
/// `draft` by `reviewer` added `when`: at its `now`, or a second past the
/// review's last change if that is later, so rounds stay in order.
///
/// # Errors
/// A change unlinked; a review of another change; a review stamped past
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
                (_, _) => return Err(BoardRuleError::NotThisTasks),
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
            let after = fields
                .updated
                .plus_seconds(1)
                .map_err(|error| BoardRuleError::Invalid(error.at("updated")))?;
            (fields.clone(), now.clone().max(after))
        }
        None => {
            let begun = ReviewFields {
                task: target.task.clone(),
                change: target.number,
                rounds: vec![],
                created: now.clone(),
                updated: now.clone(),
            };
            (begun, now.clone())
        }
    };
    next.rounds.push(Round {
        commit: draft.commit,
        reviewer: reviewer.clone(),
        at: at.clone(),
        verdict: draft.verdict,
        findings: draft.findings,
    });
    next.updated = at;
    Review::new(next).map_err(BoardRuleError::Invalid)
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
