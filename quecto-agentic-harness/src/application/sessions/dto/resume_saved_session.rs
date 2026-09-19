//! Boundary values of Resume saved session (#1863, D8 #1977): the target a
//! client named, admitted by affirmative rules; what a resumed session is;
//! what a startup open yields; and why a resume was refused or failed.
use super::{SaveSessionError, SessionTransitionRefused};
use crate::application::sessions::conversation_ledger::LedgerAdvance;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;
use crate::domain::workflow::WorkflowRunPersisted;
#[path = "resume_disposition.rs"]
mod resume_disposition;
use super::{ResumeDecision, StartupRefusal};
use crate::domain::resume_decision::ResumeAction;
pub use resume_disposition::ResumeDisposition;
#[path = "resume_target.rs"]
mod resume_target;
pub use resume_target::ResumeTarget;
#[path = "resume_refusal_code.rs"]
mod resume_refusal_code;
#[path = "resume_refusal_text.rs"]
mod resume_refusal_text;

/// A saved session was resumed: the departing one was settled and saved,
/// the target claimed and loaded, its history and workflow restored, and
/// `identity` — claimed — installed everywhere the key is held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSessionResumed {
    /// The name as the client spelled it.
    pub name: String,
    pub identity: SessionIdentity,
    /// The live conversation's length after the restore, injected prompt
    /// included (what the acknowledgement has always reported).
    pub message_count: usize,
    /// The ledger position after the switch, for the transports to announce.
    pub ledger: LedgerAdvance,
}

/// What opening the loop's session at startup yielded: the persisted
/// conversation (empty for a new or ephemeral session) and the workflow run
/// recorded with it, if any.
#[derive(Debug, Clone)]
pub struct StartupSessionOpened {
    pub messages: Vec<Message>,
    pub workflow_run: Option<WorkflowRunPersisted>,
}

/// Why no saved session was resumed. Every failure precedes the key
/// replacement: the current session, its key, its conversation and its
/// roster records are kept; a claim taken on another key is released, the
/// loop's own never (#1995); children the fleet already settled stay settled.
#[derive(Debug)]
pub enum ResumeSavedSessionError {
    /// The agent is running a turn: the interface admits no resume (#2011).
    Busy,
    /// A `--no-session` loop resumes nothing.
    Ephemeral,
    /// The target is not one of the accepted spellings.
    InvalidName,
    /// The target exists and its home does not admit a restore here (#2011).
    Decision(Box<ResumeDecision>),
    /// The home changed since the client was shown it (#2011).
    StaleHomeVersion,
    /// This runtime's own execution directory cannot be discovered (#2011).
    CurrentScopeUnavailable(String),
    /// An explicit action must name the home version it was decided on.
    HomeVersionRequired(ResumeAction),
    /// No executor of `action` is composed; nothing else was done instead.
    ActionUnavailable {
        action: ResumeAction,
        reason: String,
    },
    /// `action` has its own transaction; this owner restores and nothing else.
    ActionExecutedElsewhere(ResumeAction),
    /// The loop's own composed session does not admit at startup (#2009).
    StartupScope(StartupRefusal),
    Refused(SessionTransitionRefused),
    /// The departing session could not be saved.
    Save(SaveSessionError),
    /// The target is owned by another live process (#1460).
    Claim(DomainError),
    /// No session is saved under the target; carries the name as spelled.
    NotFound(String),
    /// The target could not be read.
    Load(DomainError),
}

impl std::error::Error for ResumeSavedSessionError {}

#[cfg(test)]
#[path = "resume_saved_session_tests.rs"]
mod tests;
