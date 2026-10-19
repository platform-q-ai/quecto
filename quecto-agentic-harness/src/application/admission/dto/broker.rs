//! The running authority's own values (#1910): the capability a client
//! presents, why the authority refuses an operation, and the operator-facing
//! status it reports. The broker's socket and links carry them.

use std::collections::BTreeMap;

use crate::domain::admission::value_objects::inference_admission::{
    AdmissionError, GroupId, GroupSnapshot, RequestId, RequestState, ScopeId,
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

/// What one settling turn of the running authority decided (#1910): the
/// broker answers waiters, notifies owners and schedules its next turn from
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    /// A failure that closed dispatch (the journal); an admission refusal is
    /// routine and is not reported.
    pub dispatch_failure: Option<AuthorityError>,
    /// Every waiting request that is no longer queued, with its answer.
    pub answers: Vec<(RequestId, Result<RequestState, AuthorityError>)>,
    /// Active attempts whose owners must stop their transport.
    pub cancellation_due: Vec<RequestId>,
    /// When the policy next needs a turn, on the authority's clock.
    pub next_wake: Result<Option<u64>, AuthorityError>,
    /// Whether the journal accepted its last write; while it has not, held
    /// work proceeds only after a successful probe.
    pub journal_healthy: bool,
}
