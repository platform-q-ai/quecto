//! Who may ask a board server what (epic #2705 B5), each rule an allowlist:
//! a caller on the Unix socket (already let in by its peer credentials) is
//! local; a caller over TCP is a device only with a held token. Reads are
//! for both; the admin routes (tokens) are for a local caller only. Like
//! every server path, nothing here panics: each failure is a value.
use std::sync::Arc;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardTokens, IssuedToken, TokenError, TokenRecord,
};
use crate::domain::project_board::value_objects::slug::Slug;

/// Who asks: a local caller, or a remote one and the token it presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller<'a> {
    Local,
    Remote(Option<&'a str>),
}

/// Who a request is from, once authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    Local,
    Device(Slug),
}

/// What a route does: read the board, or administer the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Admin,
}

/// Why a request is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// No token, or one the server does not hold (unknown or revoked).
    Token,
    /// An admin route asked over TCP.
    LocalOnly,
    /// The tokens could not be read.
    Unavailable(TokenError),
}

/// The server's authentication, authorization and token administration.
pub struct BoardAccess {
    tokens: Arc<dyn BoardTokens>,
    clock: Arc<dyn BoardClock>,
}

impl BoardAccess {
    pub fn new(tokens: Arc<dyn BoardTokens>, clock: Arc<dyn BoardClock>) -> Self {
        Self { tokens, clock }
    }

    /// Who `caller` is.
    /// # Errors
    /// A remote caller with no held token, or tokens that cannot be read.
    pub fn authenticate(&self, caller: Caller<'_>) -> Result<Principal, Refused> {
        match caller {
            Caller::Local => Ok(Principal::Local),
            Caller::Remote(None) => Err(Refused::Token),
            Caller::Remote(Some(presented)) => match self.tokens.verify(presented) {
                Ok(Some(name)) => Ok(Principal::Device(name)),
                Ok(None) => Err(Refused::Token),
                Err(error) => Err(Refused::Unavailable(error)),
            },
        }
    }

    /// Whether `principal` may use a route of `scope`.
    /// # Errors
    /// A device asking an admin route.
    pub fn authorize(principal: &Principal, scope: Scope) -> Result<(), Refused> {
        match (principal, scope) {
            (Principal::Local, Scope::Read | Scope::Admin)
            | (Principal::Device(_), Scope::Read) => Ok(()),
            (Principal::Device(_), Scope::Admin) => Err(Refused::LocalOnly),
        }
    }

    /// Makes a token named `name`, made now.
    /// # Errors
    /// A name that is no slug, one taken, a full store, or the clock or
    /// store failed.
    pub fn add(&self, name: &str) -> Result<IssuedToken, TokenError> {
        let name = named(name)?;
        let now = self.clock.now().map_err(|error| {
            TokenError::Unavailable(ProjectBoardError::Clock(error).to_string())
        })?;
        self.tokens.add(&name, &now)
    }

    /// Revokes the token named `name`, giving the name.
    /// # Errors
    /// A name that is no slug, no such token, or the store failed.
    pub fn revoke(&self, name: &str) -> Result<Slug, TokenError> {
        let name = named(name)?;
        self.tokens.revoke(&name).map(|()| name)
    }

    /// The tokens held: names and dates, never tokens.
    /// # Errors
    /// The store failed.
    pub fn list(&self) -> Result<Vec<TokenRecord>, TokenError> {
        self.tokens.list()
    }
}

/// `name` as a token's name.
fn named(name: &str) -> Result<Slug, TokenError> {
    Slug::parse(name).map_err(|_| {
        TokenError::BadName(format!(
            "a token name is 1 to 64 of a-z, 0-9 and inner '-' (got '{}')",
            name.escape_debug()
        ))
    })
}

#[cfg(test)]
#[path = "board_access_tests.rs"]
mod tests;
