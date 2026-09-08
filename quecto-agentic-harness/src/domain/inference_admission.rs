//! Deterministic admission vocabulary and validated, immutable policy inputs.
//!
//! Times are monotonic milliseconds supplied by the authority. No provider,
//! process, transport or credential identity belongs in this module.
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroupId(String);

impl GroupId {
    pub fn new(value: &str) -> Result<Self, AdmissionError> {
        if value.is_empty() || value.len() > 256 {
            return Err(AdmissionError::InvalidConfig);
        }
        Ok(Self(value.into()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScopeId {
    pub epoch: u64,
    pub serial: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestId {
    pub scope: ScopeId,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadClass {
    Interactive,
    Background,
}

#[derive(Debug, Clone)]
pub struct GroupPolicy {
    pub capacity: usize,
    pub reserve: usize,
    pub min_interval_ms: u64,
    pub queue_capacity: usize,
    pub queue_timeout_ms: u64,
    pub attempt_timeout_ms: u64,
    /// Operator-configured initial no-hint throttle delay, in milliseconds.
    pub fallback_base_ms: u64,
    pub max_cooldown_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AdmissionConfig {
    pub groups: BTreeMap<GroupId, GroupPolicy>,
    pub aliases: BTreeMap<String, GroupId>,
    /// Lifetime issued-scope bound, including retired replay fences.
    pub max_scopes: usize,
    pub terminal_capacity: usize,
}

impl AdmissionConfig {
    pub fn validate(&self) -> Result<(), AdmissionError> {
        if self.groups.is_empty()
            || self.aliases.is_empty()
            || self.max_scopes == 0
            || self.terminal_capacity == 0
            || self.aliases.iter().any(|(alias, group)| {
                alias.is_empty() || alias.len() > 256 || !self.groups.contains_key(group)
            })
            || self.groups.iter().any(|(group, p)| {
                p.capacity == 0
                    || p.reserve >= p.capacity
                    || p.min_interval_ms == 0
                    || p.queue_capacity == 0
                    || p.queue_timeout_ms == 0
                    || p.attempt_timeout_ms == 0
                    || p.fallback_base_ms == 0
                    || p.fallback_base_ms > p.max_cooldown_ms
                    || p.max_cooldown_ms == 0
                    || !self.aliases.values().any(|mapped| mapped == group)
            })
        {
            return Err(AdmissionError::InvalidConfig);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    InvalidConfig,
    UnknownGroup,
    UnknownScope,
    StaleEpoch,
    ScopeLimit,
    UnknownRequest,
    Replay,
    Conflict,
    QueueFull,
    Unavailable,
    TimeRegression,
    Busy,
    /// The group holds uncertain (unacknowledged) occupancy; no grant is safe.
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOutcome {
    Finished,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    Queued {
        deadline: u64,
    },
    Active {
        started: u64,
        deadline: u64,
        cancellation_required: bool,
    },
    Terminal(TerminalOutcome),
}

/// Receipt-anchored advice. Reporting it does not complete the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThrottleFeedback {
    /// Typed throttle without usable advice; randomness supplied by authority.
    NoHint {
        jitter: u64,
    },
    Until(u64),
    Unavailable,
}

/// A confirmed transport completion, not receipt/receiver creation. P2 owns
/// header normalization and fallback; every completion carries feedback once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feedback {
    Success,
    Failure,
    Throttle { delay_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupSnapshot {
    /// Includes uncertain and orphaned occupancy: capacity is never reclaimed silently.
    pub active: usize,
    pub queued: usize,
    /// Abandoned or restart-orphaned attempts still counted as active.
    pub uncertain: usize,
    pub cooldown_until: u64,
    pub unavailable: bool,
    pub observed_at: u64,
}

/// Outcome of a client disappearing: queued work is cancelled, active work is
/// retained as uncertain occupancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AbandonReport {
    pub cancelled: usize,
    pub uncertain: usize,
}

/// One possibly outstanding remote attempt. Scope is the serial only: the
/// epoch is the ledger's, and no capability material is ever recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutstandingAttempt {
    pub group: GroupId,
    pub scope: u64,
    pub sequence: u64,
}

/// Durable per-group timing relative to the ledger's observation instant, so a
/// restarted authority can rehydrate against a fresh monotonic clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedgerGroup {
    pub cooldown_remaining_ms: u64,
    pub pacing_remaining_ms: u64,
    pub unavailable: bool,
}

/// The minimum state an authority must persist before a grant is visible:
/// accounting epoch, outstanding occupancy and group deadlines. Queue payloads
/// are deliberately absent; they are cancelled by any restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionLedger {
    pub epoch: u64,
    pub outstanding: Vec<OutstandingAttempt>,
    pub groups: BTreeMap<GroupId, LedgerGroup>,
}
