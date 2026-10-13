//! What a kill or removal claim requires of the record it claims. The
//! condition is checked in the same critical section that grants the
//! claim, so a ref reused between a listing and a kill can never be
//! claimed on the earlier record's word.

use std::fmt;

use super::{
    EnvironmentLookupError, EnvironmentRecord, EnvironmentRegistry, EnvironmentRegistryState,
    KillClaim,
};

/// The longest creator key, in bytes.
pub const MAX_CREATOR_KEY_BYTES: usize = 256;

/// A session key a record's CREATED-BY is matched against: 1 to
/// [`MAX_CREATOR_KEY_BYTES`] bytes of printable ASCII other than space
/// (`cli:pocket-7`, `telegram:42`, `chat-…`). Never empty, so a condition
/// on it never admits a session-less record (whose CREATED-BY is empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorKey(String);

impl CreatorKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CreatorKey {
    type Error = InvalidCreatorKey;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        let bytes = key.as_bytes();
        let length_admitted = (1..=MAX_CREATOR_KEY_BYTES).contains(&bytes.len());
        let shape_admitted = bytes.iter().all(u8::is_ascii_graphic);
        if length_admitted && shape_admitted {
            return Ok(Self(key));
        }
        Err(InvalidCreatorKey)
    }
}

/// A creator key outside the allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCreatorKey;

impl fmt::Display for InvalidCreatorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a creator key is 1-256 printable ASCII characters with no spaces")
    }
}

impl std::error::Error for InvalidCreatorKey {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimCondition {
    /// Any record under the ref.
    Any,
    /// Only a record whose CREATED-BY is exactly this key.
    CreatedBy(CreatorKey),
}

impl ClaimCondition {
    /// Admit `record` or say why not: `Any` admits every record,
    /// `CreatedBy` only an exact, whole-string match (no prefix, no
    /// trimming, no case folding).
    pub fn judge(&self, record: &EnvironmentRecord) -> Result<(), CreatorMismatch> {
        self.judge_creator(&record.environment_ref, &record.created_by)
    }

    /// [`Self::judge`] of the environment `environment_ref` created by
    /// `created_by`, for a record no longer held (one a restore forgot).
    pub fn judge_creator(
        &self,
        environment_ref: &str,
        created_by: &str,
    ) -> Result<(), CreatorMismatch> {
        match self {
            Self::Any => Ok(()),
            Self::CreatedBy(key) if created_by == key.as_str() => Ok(()),
            Self::CreatedBy(key) => Err(CreatorMismatch {
                environment_ref: environment_ref.to_string(),
                expected: key.as_str().to_string(),
                created_by: created_by.to_string(),
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

impl ClaimRefusal {
    /// The refusal of an unconditional claim: only ever a lookup refusal,
    /// since [`ClaimCondition::Any`] admits every record.
    pub(super) fn unconditional(self) -> EnvironmentLookupError {
        match self {
            Self::Lookup(error) => error,
            Self::CreatedByOther(mismatch) => {
                unreachable!("ClaimCondition::Any refused a creator: {mismatch:?}")
            }
        }
    }
}

impl EnvironmentRegistry {
    /// The exclusive kill claim (see [`Self::begin_kill`]) granted only
    /// when `condition` admits the record
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
