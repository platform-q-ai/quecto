//! The running authority's own values (#1910): the capability a client
//! presents, why the authority refuses an operation, and the operator-facing
//! status it reports. The broker's socket and links carry them.

use std::collections::BTreeMap;

use crate::domain::admission::value_objects::inference_admission::{
    AdmissionConfig, AdmissionError, GroupId, GroupSnapshot, RequestId, RequestState, ScopeId,
    UncertainHold,
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
    /// A queued wait expired while uncertain leases held its group (#2664):
    /// the waiter is told which, and when they clear by themselves.
    HeldByUncertainLease(UncertainHold),
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
    /// When the policy next needs a turn, on the authority's clock: `None`
    /// when nothing is pending, an error when it cannot say.
    pub next_wake: Result<Option<u64>, AuthorityError>,
    /// Whether the journal accepted its last write; while it has not, held
    /// work proceeds only after a successful probe.
    pub journal_healthy: bool,
}

/// How soon a turn retries when the policy cannot say when it next needs one.
const WAKE_RETRY_MS: u64 = 50;
/// How often held work probes the journal during an outage: it can proceed
/// only after a successful write, so it does not wait for queue deadlines.
const JOURNAL_PROBE_INTERVAL_MS: u64 = 250;

impl Settlement {
    /// When the broker's next turn is due, for a timer armed at `armed_at`
    /// on the authority's clock (#1910 review): the policy's wake, a retry
    /// soon after arming when the policy cannot say, and during a journal
    /// outage no later than one probe interval after arming. Measured from
    /// arming, not from the turn's start, so a slow write cannot leave the
    /// probe already due and spin.
    pub fn wake_at(&self, armed_at: u64) -> Option<u64> {
        let wake = match &self.next_wake {
            Ok(wake) => *wake,
            Err(_) => Some(armed_at.saturating_add(WAKE_RETRY_MS)),
        };
        if self.journal_healthy {
            return wake;
        }
        let probe = armed_at.saturating_add(JOURNAL_PROBE_INTERVAL_MS);
        Some(wake.map_or(probe, |wake| wake.min(probe)))
    }
}

/// How the running authority starts (#1910): its policy, whether its
/// directory shows prior operation, and whether the operator accepted that
/// a ledger missing after prior operation starts empty.
#[derive(Debug, Clone)]
pub struct AuthorityStart {
    pub policy: AdmissionConfig,
    pub previously_operated: bool,
    pub accept_missing_ledger: bool,
}

/// Why the running authority could not start from its journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreError {
    /// The journal could not be read, or its ledger is corrupt.
    Unreadable(String),
    /// The ledger is missing after prior operation, and the operator has not
    /// accepted starting empty: outstanding remote work cannot be accounted.
    MissingLedger,
    /// The ledger does not fit the policy.
    Incompatible(AuthorityError),
    /// The initial ledger could not be made durable.
    NotDurable(AuthorityError),
}
