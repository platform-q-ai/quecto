//! The swarm coordination board's pure decisions (#2265, #2266): value
//! records, the policy ported from `swarm_policy.py`, and input validation.
//! No I/O and no storage format.
pub mod policy;
pub mod records;
pub mod validation;

pub use policy::{
    Access, PROPOSED_OUTCOMES, STOP_STATUSES, admission, authorize, completion, describe, expired,
    require_budget, require_unsubmitted, resume_blockers, revalidation, validate_extension,
};
pub use records::{
    Criterion, CriterionKind, EvidenceRef, EvidenceRow, MemberRecord, MemberState, RunRecord,
    RunState, TaskRecord, TaskState,
};
pub use validation::{bounded, bounded_text, criteria};

/// A refusal by the board. `Display` is the exact message members see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardError(pub String);

impl BoardError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BoardError {}
