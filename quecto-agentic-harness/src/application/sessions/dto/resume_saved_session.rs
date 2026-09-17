//! Boundary values of Resume saved session (#1863, D8 #1977): the target
//! a client named on the wire, admitted by affirmative rules; what a
//! resumed session is; what a startup open yields; and why a resume was
//! refused or failed.
use super::{SaveSessionError, SessionTransitionRefused};
use crate::application::sessions::conversation_ledger::LedgerAdvance;
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session::USER_CHAT_PREFIX;
use crate::domain::session_identity::SessionIdentity;
use crate::domain::workflow::WorkflowRunPersisted;

#[path = "resume_disposition.rs"]
mod resume_disposition;
pub use resume_disposition::ResumeDisposition;

/// The saved session a client asked to resume: the name as the client
/// spelled it (trimmed; echoed in the acknowledgement and the not-found
/// refusal) and the identity it denotes.
///
/// The accepted variants, each admitted affirmatively: a full user-chat key
/// (`chat-…`, the `/resume` picker's selection), an already-qualified legacy
/// `cli:<name>` key (kept as it is, never re-prefixed), and a typed legacy
/// `<name>` (a `cli:<name>` session). Every other spelling is refused; the
/// allowlist is the domain's session-name allowlist and admits no new
/// character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeTarget {
    pub name: String,
    pub identity: SessionIdentity,
}

impl ResumeTarget {
    pub fn parse(raw: &str) -> Result<Self, ResumeSavedSessionError> {
        let name = raw.trim();
        let identity = if let Some(suffix) = name.strip_prefix("cli:")
            && SessionIdentity::is_valid_cli_name(suffix)
        {
            SessionIdentity::named_cli(suffix)
        } else if name.starts_with(USER_CHAT_PREFIX) {
            SessionIdentity::user_chat(name)
        } else {
            SessionIdentity::named_cli(name)
        }
        .map_err(|_| ResumeSavedSessionError::InvalidName)?;
        Ok(Self {
            name: name.to_string(),
            identity,
        })
    }
}

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
/// roster records are kept; a claim taken on the target is released;
/// children the fleet already settled stay settled.
#[derive(Debug)]
pub enum ResumeSavedSessionError {
    /// A `--no-session` loop resumes nothing.
    Ephemeral,
    /// The target is not one of the accepted spellings.
    InvalidName,
    Scope(ResumeDisposition),
    /// The loop's own composed session does not admit at startup (#2009):
    /// carries the key as composed, so the refusal can say what to do now.
    StartupScope {
        key: String,
        disposition: ResumeDisposition,
    },
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

impl std::fmt::Display for ResumeSavedSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ephemeral => f.write_str("cannot resume sessions in ephemeral mode"),
            Self::InvalidName => {
                f.write_str("session name must contain only alphanumeric, '-', or '_'")
            }
            Self::Scope(disposition) => write!(
                f,
                "session resume unavailable: {disposition}; Cancel (open/fork/locate are unavailable)"
            ),
            Self::StartupScope { key, disposition } => match disposition {
                ResumeDisposition::LegacyUnscoped => write!(
                    f,
                    "session '{key}' cannot start here: it predates workspace scoping and has \
                     no home, and history is never associated implicitly. Start a new \
                     session under another name with `-s <name>` (or `--no-session` for an \
                     ephemeral run); the old transcript stays in place and visible in the \
                     Global list of /resume. Explicit association of a legacy session with \
                     a folder arrives in a later slice (#2014)."
                ),
                ResumeDisposition::DifferentExecutionDirectory => write!(
                    f,
                    "session '{key}' cannot start here: {disposition}. Start it from that \
                     directory, or start a new session under another name with `-s <name>`."
                ),
                ResumeDisposition::HomeChanged | ResumeDisposition::Unavailable(_) => write!(
                    f,
                    "session '{key}' cannot start here: {disposition}. Start a new session \
                     under another name with `-s <name>`; the transcript is preserved."
                ),
            },
            Self::Refused(refused) => write!(f, "{refused}"),
            Self::Save(error) => write!(f, "failed to save current session: {error}"),
            Self::Claim(error) => write!(f, "{error}"),
            Self::NotFound(name) => write!(f, "session not found: {name}"),
            Self::Load(error) => write!(f, "failed to load session: {error}"),
        }
    }
}

impl std::error::Error for ResumeSavedSessionError {}

#[cfg(test)]
#[path = "resume_saved_session_tests.rs"]
mod tests;
