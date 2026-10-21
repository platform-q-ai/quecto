//! #2659: a refusal names a status with the article it reads with.
use super::*;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskStatus};

#[test]
fn a_status_is_named_with_its_article() {
    for (refusal, said) in [
        (
            BoardRuleError::PlanNotDraft(PlanState::Accepted),
            "an accepted plan is not a draft: only drafts are edited, accepted or discarded",
        ),
        (
            BoardRuleError::PlanNotDraft(PlanState::Superseded),
            "a superseded plan is not a draft: only drafts are edited, accepted or discarded",
        ),
        (
            BoardRuleError::NotDraft(TaskStatus::InProgress),
            "an in_progress task is not a draft: only drafts are edited",
        ),
        (
            BoardRuleError::NotDraft(TaskStatus::Ready),
            "a ready task is not a draft: only drafts are edited",
        ),
        (
            BoardRuleError::TaskFinished(TaskStatus::Archived),
            "an archived task's plans are settled",
        ),
        (
            BoardRuleError::NotTakeable(TaskStatus::Archived),
            "an archived task cannot be taken over",
        ),
        (
            BoardRuleError::NotHolderExpired,
            "only the claim's holder may do this, and the task's claim has expired: take the \
             task over first with take_over(task)",
        ),
    ] {
        assert_eq!(refusal.to_string(), said);
    }
}
