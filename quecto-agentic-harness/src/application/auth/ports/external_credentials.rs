//! Credentials another tool already stored (#1840): the OAuth entries a
//! login can import instead of signing in afresh.

use crate::domain::auth::provider::LoginProvider;

/// One OAuth credential found in the external store, as it states it.
#[derive(Clone, PartialEq, Eq)]
pub struct ExternalCredential {
    pub provider: LoginProvider,
    /// Empty when the store holds none.
    pub access_token: String,
    /// Empty when the store holds none.
    pub refresh_token: String,
    /// The token's server-side expiry as the store states it, Unix seconds
    /// (0 when unstated). Not a credential's `expires_at`: the import turns
    /// it into one, less the margin.
    pub stated_expiry_secs: i64,
    /// The account the access token names, when its provider names one
    /// (OpenAI's `chatgpt_account_id`).
    pub account_id: Option<String>,
}

impl std::fmt::Debug for ExternalCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalCredential")
            .field("provider", &self.provider)
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("stated_expiry_secs", &self.stated_expiry_secs)
            .field("account_id", &self.account_id)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalCredentialsError {
    /// There is no external store at `path`.
    NotFound { path: String },
    /// The store at `path` could not be read, in the reader's words.
    Unreadable { path: String, reason: String },
    /// The store could not be parsed, in the parser's words.
    Malformed(String),
}

pub trait ExternalCredentialSource: Send + Sync {
    /// Every OAuth credential the external store holds for a login
    /// provider, in the order an import takes them (Anthropic, then
    /// OpenAI). Reading writes nothing.
    fn load(&self) -> Result<Vec<ExternalCredential>, ExternalCredentialsError>;
}
