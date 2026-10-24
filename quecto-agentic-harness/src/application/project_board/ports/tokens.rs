//! The board server's device tokens (epic #2705 B5): a bearer token per
//! TCP device, shown once; an adapter keeps only digests.
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The most tokens a server holds.
pub const MAX_TOKENS: usize = 256;

/// A token as listed: its name and when it was made, never the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    pub name: Slug,
    pub created: Timestamp,
}

/// A token as made: its secret is shown once, and never kept or logged.
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedToken {
    pub record: TokenRecord,
    pub secret: String,
}

impl std::fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IssuedToken({:?}, <secret>)", self.record)
    }
}

/// Why a token could not be made, revoked, listed or checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    BadName(String),
    Exists(Slug),
    NoSuchToken(Slug),
    Full,
    Unavailable(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadName(why) | Self::Unavailable(why) => f.write_str(why),
            Self::Exists(name) => {
                write!(
                    f,
                    "a token named '{}' exists: revoke it first",
                    name.as_str()
                )
            }
            Self::NoSuchToken(name) => write!(f, "no token is named '{}'", name.as_str()),
            Self::Full => write!(f, "the server holds {MAX_TOKENS} tokens: revoke one first"),
        }
    }
}

/// Where a server's device tokens are kept (a file; SQLite later).
pub trait BoardTokens: Send + Sync {
    /// Makes a token named `name`, made at `created`, keeping its digest.
    /// # Errors
    /// The name is taken, [`MAX_TOKENS`] are held, or the store failed.
    fn add(&self, name: &Slug, created: &Timestamp) -> Result<IssuedToken, TokenError>;

    /// Revokes the token named `name`: it is refused from now on.
    /// # Errors
    /// No token has the name, or the store failed.
    fn revoke(&self, name: &Slug) -> Result<(), TokenError>;

    /// The tokens held.
    /// # Errors
    /// The store failed.
    fn list(&self) -> Result<Vec<TokenRecord>, TokenError>;

    /// The name of the held token `presented` is, or `None`.
    /// # Errors
    /// The store failed.
    fn verify(&self, presented: &str) -> Result<Option<Slug>, TokenError>;
}
