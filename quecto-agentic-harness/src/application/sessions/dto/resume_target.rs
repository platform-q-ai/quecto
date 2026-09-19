//! The exact target of a resume request (#1863, #2011): admitted by
//! affirmative rules, never by prefix or fuzzy match.
use super::ResumeSavedSessionError;
use crate::domain::session::USER_CHAT_PREFIX;
use crate::domain::session_identity::SessionIdentity;

/// The saved session a client asked to resume: the name as the client
/// spelled it (trimmed; echoed in the acknowledgement and the not-found
/// refusal) and the identity it denotes. The accepted variants, each
/// admitted affirmatively: a full user-chat key (`chat-…`, the `/resume`
/// picker's selection), an already-qualified legacy `cli:<name>` key (kept
/// as it is, never re-prefixed), and a typed legacy `<name>` (a `cli:<name>`
/// session). Every other spelling is refused; the allowlist is the domain's
/// session-name allowlist and admits no new character.
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
