//! What a kill or removal claim requires of the record it claims. The
//! condition is checked in the same critical section that grants the
//! claim, so a ref reused between a listing and a kill can never be
//! claimed on the earlier record's word.

use super::{EnvironmentLookupError, EnvironmentRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimCondition {
    /// Any record under the ref.
    Any,
    /// Only a record whose CREATED-BY is exactly this key.
    CreatedBy(String),
}

impl ClaimCondition {
    /// Admit `record` or say why not: `Any` admits every record,
    /// `CreatedBy` only an exact, whole-string match (no prefix, no
    /// trimming).
    pub fn judge(&self, record: &EnvironmentRecord) -> Result<(), CreatorMismatch> {
        match self {
            Self::Any => Ok(()),
            Self::CreatedBy(key) if record.created_by == *key => Ok(()),
            Self::CreatedBy(key) => Err(CreatorMismatch {
                environment_ref: record.environment_ref.clone(),
                expected: key.clone(),
                created_by: record.created_by.clone(),
            }),
        }
    }

    /// Whether [`Self::judge`] admits `record`.
    pub fn admits(&self, record: &EnvironmentRecord) -> bool {
        self.judge(record).is_ok()
    }
}

/// A record created by another key than the condition required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorMismatch {
    pub environment_ref: String,
    /// The key the condition required.
    pub expected: String,
    /// The record's CREATED-BY (empty for a session-less run).
    pub created_by: String,
}

/// Why a conditional claim was refused. Either way nothing was claimed
/// and the record is as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimRefusal {
    /// The ordinary claim's own refusal (unknown, stopped, stale).
    Lookup(EnvironmentLookupError),
    /// The record under the ref was created by another key.
    CreatedByOther(CreatorMismatch),
}

#[cfg(test)]
#[path = "claim_condition_tests.rs"]
mod tests;
