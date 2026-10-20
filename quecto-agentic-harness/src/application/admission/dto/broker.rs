//! The running authority's own values (#1910): the capability a client
//! presents, why the authority refuses an operation, and the operator-facing
//! status it reports. The broker's socket and links carry them.

use std::collections::BTreeMap;

use crate::domain::admission::value_objects::inference_admission::{
    AdmissionError, GroupId, GroupSnapshot, ScopeId,
};

/// Scope identity plus the opaque secret proving the caller was issued it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub scope: ScopeId,
    pub token: String,
}

/// Why the authority refused an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityError {
    Unauthorized,
    Admission(AdmissionError),
    JournalUnavailable,
}

impl From<AdmissionError> for AuthorityError {
    fn from(error: AdmissionError) -> Self {
        Self::Admission(error)
    }
}

/// Operator-facing state; contains no capability material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityStatus {
    pub epoch: u64,
    pub journal_healthy: bool,
    /// Scopes holding a live capability (roots and descendants).
    pub live_scopes: usize,
    pub groups: BTreeMap<GroupId, GroupSnapshot>,
}
