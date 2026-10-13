//! What a kill or removal claim requires of the record it claims. The
//! condition is checked in the same critical section that grants the
//! claim, so a ref reused between a listing and a kill can never be
//! claimed on the earlier record's word.

use super::{
    EnvironmentLookupError, EnvironmentRecord, EnvironmentRegistry, EnvironmentRegistryState,
    KillClaim,
};

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

impl EnvironmentRegistry {
    /// [`Self::begin_kill`] granted only when `condition` admits the record
    /// under the ref, judged in the same critical section that grants the
    /// claim: a ref reused since the caller last looked is never claimed on
    /// the earlier record's word. A refusal claims nothing and writes
    /// nothing.
    pub fn begin_kill_if(
        &self,
        environment_ref: &str,
        condition: &ClaimCondition,
    ) -> Result<KillClaim, ClaimRefusal> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::admitted_locked(&state, environment_ref, condition)?;
        self.claim_kill_locked(state, environment_ref)
            .map_err(ClaimRefusal::Lookup)
    }

    /// Whether `condition` admits the record under `environment_ref`, with
    /// the state lock held by the caller that goes on to claim it.
    pub(super) fn admitted_locked(
        state: &EnvironmentRegistryState,
        environment_ref: &str,
        condition: &ClaimCondition,
    ) -> Result<(), ClaimRefusal> {
        let record = state.entries.get(environment_ref).ok_or_else(|| {
            ClaimRefusal::Lookup(EnvironmentLookupError::Unknown(environment_ref.to_string()))
        })?;
        condition
            .judge(record)
            .map_err(ClaimRefusal::CreatedByOther)
    }
}

#[cfg(test)]
#[path = "claim_condition_tests.rs"]
mod tests;
