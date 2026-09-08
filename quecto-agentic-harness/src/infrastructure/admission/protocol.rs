//! Versioned, bounded, framed JSON protocol of the admission authority.
//! Separate from the agent-control protocol; carries no prompts or provider
//! credentials. Every message is one `quecto-line-io` frame.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::application::ports::{AuthorityError, AuthorityStatus, Credential};
use crate::domain::inference_admission::*;
use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;

pub const PROTOCOL_VERSION: u8 = 1;
/// Requests and replies are small; a large declared length is an attack, not a
/// message. The hello reply (the effective policy) must fit as well; the server
/// validates that at start.
pub const FRAME_CAP: usize = 256 * 1024;
/// Once a frame prefix has arrived, the rest must follow within this bound.
pub const FRAME_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);
pub const CAPABILITY_DIRECT: &str = "admission-uds-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeWire {
    pub epoch: u64,
    pub serial: u64,
}

impl From<ScopeId> for ScopeWire {
    fn from(scope: ScopeId) -> Self {
        Self {
            epoch: scope.epoch,
            serial: scope.serial,
        }
    }
}
impl From<ScopeWire> for ScopeId {
    fn from(scope: ScopeWire) -> Self {
        Self {
            epoch: scope.epoch,
            serial: scope.serial,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassWire {
    Interactive,
    Background,
}
impl From<ClassWire> for WorkloadClass {
    fn from(class: ClassWire) -> Self {
        match class {
            ClassWire::Interactive => Self::Interactive,
            ClassWire::Background => Self::Background,
        }
    }
}
impl From<WorkloadClass> for ClassWire {
    fn from(class: WorkloadClass) -> Self {
        match class {
            WorkloadClass::Interactive => Self::Interactive,
            WorkloadClass::Background => Self::Background,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThrottleWire {
    NoHint { jitter: u64 },
    Until { deadline_ms: u64 },
    Unavailable,
}
impl From<ThrottleFeedback> for ThrottleWire {
    fn from(f: ThrottleFeedback) -> Self {
        match f {
            ThrottleFeedback::NoHint { jitter } => Self::NoHint { jitter },
            ThrottleFeedback::Until(deadline_ms) => Self::Until { deadline_ms },
            ThrottleFeedback::Unavailable => Self::Unavailable,
        }
    }
}
impl From<ThrottleWire> for ThrottleFeedback {
    fn from(f: ThrottleWire) -> Self {
        match f {
            ThrottleWire::NoHint { jitter } => Self::NoHint { jitter },
            ThrottleWire::Until { deadline_ms } => Self::Until(deadline_ms),
            ThrottleWire::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedbackWire {
    Success,
    Failure,
    Throttle { delay_ms: u64 },
}
impl From<Feedback> for FeedbackWire {
    fn from(f: Feedback) -> Self {
        match f {
            Feedback::Success => Self::Success,
            Feedback::Failure => Self::Failure,
            Feedback::Throttle { delay_ms } => Self::Throttle { delay_ms },
        }
    }
}
impl From<FeedbackWire> for Feedback {
    fn from(f: FeedbackWire) -> Self {
        match f {
            FeedbackWire::Success => Self::Success,
            FeedbackWire::Failure => Self::Failure,
            FeedbackWire::Throttle { delay_ms } => Self::Throttle { delay_ms },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateWire {
    Queued {
        deadline_ms: u64,
    },
    Active {
        started_ms: u64,
        deadline_ms: u64,
        cancellation_required: bool,
    },
    Finished,
    Cancelled,
    TimedOut,
}
impl From<RequestState> for StateWire {
    fn from(state: RequestState) -> Self {
        match state {
            RequestState::Queued { deadline } => Self::Queued {
                deadline_ms: deadline,
            },
            RequestState::Active {
                started,
                deadline,
                cancellation_required,
            } => Self::Active {
                started_ms: started,
                deadline_ms: deadline,
                cancellation_required,
            },
            RequestState::Terminal(TerminalOutcome::Finished) => Self::Finished,
            RequestState::Terminal(TerminalOutcome::Cancelled) => Self::Cancelled,
            RequestState::Terminal(TerminalOutcome::TimedOut) => Self::TimedOut,
        }
    }
}
impl From<StateWire> for RequestState {
    fn from(state: StateWire) -> Self {
        match state {
            StateWire::Queued { deadline_ms } => Self::Queued {
                deadline: deadline_ms,
            },
            StateWire::Active {
                started_ms,
                deadline_ms,
                cancellation_required,
            } => Self::Active {
                started: started_ms,
                deadline: deadline_ms,
                cancellation_required,
            },
            StateWire::Finished => Self::Terminal(TerminalOutcome::Finished),
            StateWire::Cancelled => Self::Terminal(TerminalOutcome::Cancelled),
            StateWire::TimedOut => Self::Terminal(TerminalOutcome::TimedOut),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupPolicyWire {
    pub capacity: usize,
    pub reserve: usize,
    pub min_interval_ms: u64,
    pub queue_capacity: usize,
    pub queue_timeout_ms: u64,
    pub attempt_timeout_ms: u64,
    pub fallback_base_ms: u64,
    pub max_cooldown_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalWire {
    pub groups: BTreeMap<String, GroupPolicyWire>,
    pub aliases: BTreeMap<String, String>,
    pub max_scopes: usize,
    pub terminal_capacity: usize,
    pub bindings: BTreeMap<String, String>,
}

impl From<&AdmissionRuntimeProposal> for ProposalWire {
    fn from(p: &AdmissionRuntimeProposal) -> Self {
        Self {
            groups: p
                .policy
                .groups
                .iter()
                .map(|(id, g)| {
                    (
                        id.as_str().to_owned(),
                        GroupPolicyWire {
                            capacity: g.capacity,
                            reserve: g.reserve,
                            min_interval_ms: g.min_interval_ms,
                            queue_capacity: g.queue_capacity,
                            queue_timeout_ms: g.queue_timeout_ms,
                            attempt_timeout_ms: g.attempt_timeout_ms,
                            fallback_base_ms: g.fallback_base_ms,
                            max_cooldown_ms: g.max_cooldown_ms,
                        },
                    )
                })
                .collect(),
            aliases: p
                .policy
                .aliases
                .iter()
                .map(|(a, g)| (a.clone(), g.as_str().to_owned()))
                .collect(),
            max_scopes: p.policy.max_scopes,
            terminal_capacity: p.policy.terminal_capacity,
            bindings: p.bindings.clone(),
        }
    }
}

impl TryFrom<ProposalWire> for AdmissionRuntimeProposal {
    type Error = AdmissionError;
    fn try_from(w: ProposalWire) -> Result<Self, AdmissionError> {
        let mut groups = BTreeMap::new();
        for (name, g) in w.groups {
            groups.insert(
                GroupId::new(&name)?,
                GroupPolicy {
                    capacity: g.capacity,
                    reserve: g.reserve,
                    min_interval_ms: g.min_interval_ms,
                    queue_capacity: g.queue_capacity,
                    queue_timeout_ms: g.queue_timeout_ms,
                    attempt_timeout_ms: g.attempt_timeout_ms,
                    fallback_base_ms: g.fallback_base_ms,
                    max_cooldown_ms: g.max_cooldown_ms,
                },
            );
        }
        let mut aliases = BTreeMap::new();
        for (alias, group) in w.aliases {
            aliases.insert(alias, GroupId::new(&group)?);
        }
        let policy = AdmissionConfig {
            groups,
            aliases,
            max_scopes: w.max_scopes,
            terminal_capacity: w.terminal_capacity,
        };
        policy.validate()?;
        Ok(Self {
            policy,
            bindings: w.bindings,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialWire {
    pub scope: ScopeWire,
    pub token: String,
}
impl From<&Credential> for CredentialWire {
    fn from(c: &Credential) -> Self {
        Self {
            scope: c.scope.into(),
            token: c.token.clone(),
        }
    }
}
impl From<CredentialWire> for Credential {
    fn from(c: CredentialWire) -> Self {
        Self {
            scope: c.scope.into(),
            token: c.token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Hello {
        version: u8,
        capability: String,
    },
    /// Roots are minted only with the owner token kept outside `client/`.
    RegisterRoot {
        class: ClassWire,
        owner_token: String,
    },
    Bind {
        credential: CredentialWire,
    },
    RegisterChild,
    Acquire {
        sequence: u64,
        alias: String,
    },
    Cancel {
        sequence: u64,
    },
    Feedback {
        sequence: u64,
        report: u64,
        feedback: ThrottleWire,
    },
    Complete {
        sequence: u64,
        feedback: FeedbackWire,
    },
    Status {
        sequence: u64,
    },
    Retire,
    /// A parent retires a descendant it registered (failed launch).
    RetireChild {
        scope: ScopeWire,
    },
    Inspect,
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub op: Op,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Client,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotWire {
    pub active: usize,
    pub queued: usize,
    pub uncertain: usize,
    pub cooldown_until_ms: u64,
    pub unavailable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthorized,
    UnsupportedProtocol,
    JournalUnavailable,
    Malformed,
    Admission,
    Cancelled,
    Timeout,
    EpochReset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Hello {
        version: u8,
        role: Role,
        epoch: u64,
        proposal: ProposalWire,
    },
    Credential {
        credential: CredentialWire,
    },
    Bound {
        next_sequence: u64,
    },
    Ok,
    State {
        state: StateWire,
    },
    Granted {
        deadline_ms: u64,
        receipt_ms: u64,
        receipt_wall_ms: u64,
    },
    Status {
        epoch: u64,
        journal_healthy: bool,
        live_scopes: usize,
        groups: BTreeMap<String, SnapshotWire>,
    },
    Reset {
        epoch: u64,
    },
    Error {
        code: ErrorCode,
        reason: String,
    },
    CancelRequired {
        sequence: u64,
    },
    Revoked,
}

/// `id` is `None` for server-initiated notices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub id: Option<u64>,
    #[serde(flatten)]
    pub body: Body,
}

pub fn error_body(error: &AuthorityError) -> Body {
    match error {
        AuthorityError::Unauthorized => Body::Error {
            code: ErrorCode::Unauthorized,
            reason: "unauthorized".into(),
        },
        AuthorityError::JournalUnavailable => Body::Error {
            code: ErrorCode::JournalUnavailable,
            reason: "ledger not durable".into(),
        },
        AuthorityError::Admission(e) => Body::Error {
            code: ErrorCode::Admission,
            reason: format!("{e:?}"),
        },
    }
}

pub fn status_body(status: &AuthorityStatus) -> Body {
    Body::Status {
        epoch: status.epoch,
        journal_healthy: status.journal_healthy,
        live_scopes: status.live_scopes,
        groups: status
            .groups
            .iter()
            .map(|(id, s)| {
                (
                    id.as_str().to_owned(),
                    SnapshotWire {
                        active: s.active,
                        queued: s.queued,
                        uncertain: s.uncertain,
                        cooldown_until_ms: s.cooldown_until,
                        unavailable: s.unavailable,
                    },
                )
            })
            .collect(),
    }
}

pub fn status_from_body(
    epoch: u64,
    journal_healthy: bool,
    live_scopes: usize,
    groups: BTreeMap<String, SnapshotWire>,
) -> Result<AuthorityStatus, AdmissionError> {
    let mut out = BTreeMap::new();
    for (name, s) in groups {
        out.insert(
            GroupId::new(&name)?,
            GroupSnapshot {
                active: s.active,
                queued: s.queued,
                uncertain: s.uncertain,
                cooldown_until: s.cooldown_until_ms,
                unavailable: s.unavailable,
                observed_at: 0,
            },
        );
    }
    Ok(AuthorityStatus {
        epoch,
        journal_healthy,
        live_scopes,
        groups: out,
    })
}
