//! The typed identity of a conversation session (#1970).
//!
//! Every session port and DTO names a session by this value, not by a raw
//! string, so the layout and key grammar stay behind the ports. It carries
//! the existing raw key only (no workspace, scope or path), read-only.
//!
//! Vocabulary, unchanged from the raw keys it wraps:
//! - the **ephemeral** identity is the empty key: a `--no-session` run, which
//!   is never persisted and whose in-run spill retention lives under the
//!   sanitized empty key exactly as before;
//! - a **named CLI** identity is `cli:<name>`, `<name>` being the affirmative
//!   session-name allowlist (ASCII alphanumeric, `_`, `-`);
//! - a **fresh chat** identity is `chat-<secs>-<uniq>` ([`user_chat_key`]); an
//!   existing one named on the wire is admitted by [`SessionIdentity::user_chat`];
//! - a **persisted** identity is whatever key a stored session carries —
//!   every historical category (`telegram:<id>`, legacy `cli:<name>`, chat
//!   keys, arbitrary keys) round-trips unchanged.
use super::error::DomainError;
use super::session::user_chat_key;

/// Opaque identity of a session: the existing raw key, read-only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionIdentity(String);

impl SessionIdentity {
    /// The identity of a run that is never persisted (the empty key).
    pub fn ephemeral() -> Self {
        Self(String::new())
    }

    /// Rebuild the identity of a persisted key. Total and unchecked on
    /// purpose: the exact bytes are kept — no trimming, case folding,
    /// validation or normalisation — so every historical key category round-trips
    /// (`persisted_key()` returns the same text), and the empty key is the
    /// ephemeral identity, the only classification the raw string ever
    /// carried. A persistence round-trip, not a wire parser: interface sites
    /// still holding a raw runtime key convert here until D2 (#1971) moves
    /// that key into `ActiveSessionState`; wire input uses [`Self::named_cli`].
    pub fn from_persisted_key(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The `cli:<name>` identity of a named session. `name` must satisfy
    /// the existing session-name allowlist: non-empty ASCII alphanumeric,
    /// `_` or `-` only.
    pub fn named_cli(name: &str) -> Result<Self, DomainError> {
        if Self::is_valid_cli_name(name) {
            Ok(Self(format!("cli:{name}")))
        } else {
            Err(DomainError::Session(
                "session name must contain only alphanumeric, '-', or '_'".to_string(),
            ))
        }
    }

    /// The affirmative session-name allowlist of [`Self::named_cli`].
    pub fn is_valid_cli_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    /// A fresh user-chat identity from the wall clock and a uniqueness token
    /// (see [`user_chat_key`]).
    pub fn fresh_chat(secs: u64, uniq: u64) -> Self {
        Self(user_chat_key(secs, uniq))
    }

    /// The identity of an existing user chat named by its full key on the
    /// wire (the `/resume` picker, D8 #1977): the `chat-` prefix followed
    /// by allowlisted session-name characters only — the same allowlist as
    /// [`Self::named_cli`]; no new characters are admitted.
    pub fn user_chat(key: &str) -> Result<Self, DomainError> {
        if key.starts_with(super::session::USER_CHAT_PREFIX) && Self::is_valid_cli_name(key) {
            Ok(Self(key.to_string()))
        } else {
            Err(DomainError::Session(
                "session name must contain only alphanumeric, '-', or '_'".to_string(),
            ))
        }
    }

    /// Whether this is the ephemeral (never persisted) identity.
    pub fn is_ephemeral(&self) -> bool {
        self.0.is_empty()
    }

    /// The key a persisted session is stored under; `None` for the
    /// ephemeral identity, which persistence refuses or no-ops.
    pub fn persisted_key(&self) -> Option<&str> {
        (!self.is_ephemeral()).then_some(self.0.as_str())
    }

    /// The raw in-run key, empty for the ephemeral identity. Read-only: this
    /// is what the runtime reported as its session key before the identity
    /// was typed (retention, audit and identity-derived labels use it).
    pub fn runtime_key(&self) -> &str {
        &self.0
    }
}

/// An identity prefix a session list may be narrowed to (#1861): the
/// existing key-prefix policy (`chat-`, `cli:`), never a filename or path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKeyPrefix(String);

impl SessionKeyPrefix {
    /// A non-empty key prefix; the empty prefix would match every session
    /// and is expressed by listing all sessions instead.
    pub fn new(prefix: &str) -> Result<Self, DomainError> {
        if prefix.is_empty() {
            return Err(DomainError::Session(
                "session key prefix must not be empty".to_string(),
            ));
        }
        Ok(Self(prefix.to_string()))
    }

    /// Whether `identity` starts with this prefix.
    pub fn admits(&self, identity: &SessionIdentity) -> bool {
        identity.runtime_key().starts_with(self.0.as_str())
    }

    /// The raw prefix text, read-only.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The id of one retained (spilled) entry of a session, as shown in collapse
/// stubs (`turn20:bash:0`) and passed back to `recall`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpillId(String);

impl SpillId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "session_identity_tests.rs"]
mod tests;
