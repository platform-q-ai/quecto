//! Who may ask a board server what (epic #2705 B5), each rule an allowlist:
//! a Unix-socket caller is local; a TCP caller is a device only with a held
//! token. Reads and writes are for both; the admin routes and the board's
//! admin operations (`bootstrap`, `clear_set_aside`) for local only. A
//! follower's replica token (B11) reads and takes replication batches,
//! which no device token may.
use std::sync::Arc;

use crate::application::project_board::dto::{BoardRequest, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardClock, BoardTokens, IssuedToken, TokenError, TokenKind, TokenRecord,
};
use crate::domain::project_board::value_objects::slug::Slug;

/// Who a request is from, once authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    Local,
    Device(Slug),
    /// A follower's token (B11).
    Replica(Slug),
}

/// How the TCP gate names a replica token's principal (a name never
/// holds a `:`).
const REPLICA: &str = "replica:";

impl Principal {
    /// Who a revision records was let in (B7): `local` or `token:<name>`
    /// (`replica:<name>` for a replica token, which writes nothing).
    pub fn recorded(&self) -> String {
        match self {
            Self::Local => "local".into(),
            Self::Device(name) => format!("token:{}", name.as_str()),
            Self::Replica(name) => format!("{REPLICA}{}", name.as_str()),
        }
    }

    /// How the TCP gate hands this principal on: a device by its token's
    /// name, a replica token as `replica:<name>`.
    pub fn gated(&self) -> String {
        match self {
            Self::Local => "local".into(),
            Self::Device(name) => name.as_str().to_string(),
            Self::Replica(name) => format!("{REPLICA}{}", name.as_str()),
        }
    }
}

/// What a request does: read the board, write it, or administer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Write,
    Admin,
    /// A follower's replication batches (B11).
    Replicate,
}

impl Scope {
    /// The scope of `request` (B7): `bootstrap` and `clear_set_aside` admin.
    pub fn of(request: &BoardRequest) -> Self {
        match (request, request.writes()) {
            (BoardRequest::Bootstrap | BoardRequest::Clear { .. }, _) => Self::Admin,
            (_, true) => Self::Write,
            (_, false) => Self::Read,
        }
    }
}

/// Why a request is refused: no held token, admin over TCP, or no tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    Token,
    LocalOnly,
    /// A device asking replication batches, kept to replica tokens (B11).
    ReplicaOnly,
    /// A replica token asking to write (B11).
    ReadOnly,
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

    /// The device (its token's name) a TCP caller presenting `token` is.
    /// # Errors
    /// No held token, or tokens that cannot be read.
    pub fn authenticate(&self, token: Option<&str>) -> Result<Slug, Refused> {
        match token.map(|token| self.tokens.verify(token)) {
            Some(Ok(Some(name))) => Ok(name),
            None | Some(Ok(None)) => Err(Refused::Token),
            Some(Err(error)) => Err(Refused::Unavailable(error)),
        }
    }

    /// The principal a TCP caller presenting `token` is: a device, or a
    /// follower's replica token (B11).
    /// # Errors
    /// No held token, or tokens that cannot be read.
    pub fn authenticate_principal(&self, token: Option<&str>) -> Result<Principal, Refused> {
        let name = self.authenticate(token)?;
        match self.tokens.kind_of(&name) {
            Ok(Some(TokenKind::Device)) => Ok(Principal::Device(name)),
            Ok(Some(TokenKind::Replica)) => Ok(Principal::Replica(name)),
            // Revoked between the two reads: no held token.
            Ok(None) => Err(Refused::Token),
            Err(error) => Err(Refused::Unavailable(error)),
        }
    }

    /// The principal the TCP gate named `name` ([`Principal::gated`]).
    /// # Errors
    /// A name no token could have.
    pub fn device(name: &str) -> Result<Principal, Refused> {
        let (principal, slug): (fn(Slug) -> Principal, &str) = match name.strip_prefix(REPLICA) {
            Some(replica) => (Principal::Replica, replica),
            None => (Principal::Device, name),
        };
        Slug::parse(slug).map(principal).map_err(|_| Refused::Token)
    }

    /// Whether `principal` may make a request of `scope`.
    /// # Errors
    /// A device asking an admin route or operation.
    pub fn authorize(principal: &Principal, scope: Scope) -> Result<(), Refused> {
        match (principal, scope) {
            (Principal::Local, Scope::Read | Scope::Write | Scope::Admin | Scope::Replicate)
            | (Principal::Device(_), Scope::Read | Scope::Write)
            | (Principal::Replica(_), Scope::Read | Scope::Replicate) => Ok(()),
            (Principal::Device(_) | Principal::Replica(_), Scope::Admin) => Err(Refused::LocalOnly),
            (Principal::Device(_), Scope::Replicate) => Err(Refused::ReplicaOnly),
            (Principal::Replica(_), Scope::Write) => Err(Refused::ReadOnly),
        }
    }

    /// Makes a token named `name`, made now.
    /// # Errors
    /// A bad or taken name, a full store, or the clock or store failed.
    pub fn add(&self, name: &str) -> Result<IssuedToken, TokenError> {
        self.add_kind(name, TokenKind::Device)
    }

    /// Makes a token of `kind` named `name`, made now (B11: a follower's
    /// is a replica token).
    /// # Errors
    /// As [`Self::add`].
    pub fn add_kind(&self, name: &str, kind: TokenKind) -> Result<IssuedToken, TokenError> {
        let name = named(name)?;
        let now = self.clock.now();
        let now = now.map_err(|e| TokenError::Unavailable(ProjectBoardError::Clock(e).to_string()));
        match kind {
            TokenKind::Device => self.tokens.add(&name, &now?),
            TokenKind::Replica => self.tokens.add_replica(&name, &now?),
        }
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
        let shown = name.escape_debug();
        TokenError::BadName(format!(
            "a token name is 1 to 64 of a-z, 0-9 and inner '-' (got '{shown}')"
        ))
    })
}

#[cfg(test)]
#[path = "board_access_tests.rs"]
mod tests;
