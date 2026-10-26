//! Replication (epic #2705 B11): a follower copies its primary's board,
//! revision by revision, with each revision's chain (a hash over every
//! revision before it, so a fork is told), the objects it reaches, the
//! epochs and the admin rows (the device tokens' digests). A follower
//! never writes the board otherwise, and never rewrites what it holds.
use super::feed::{BatchLimit, BoardStream, OpenRefused};
use super::{BoardStoreError, TokenError, TokenKind};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The most bytes of objects and revisions a batch carries past its first
/// row (a batch always carries one, so it always makes progress): what a
/// follower holds in memory for one batch is about this much.
pub const BATCH_BYTES: usize = 8 * 1024 * 1024;

/// One revision as the primary holds it, and its chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaRevision {
    pub head: String,
    pub root: String,
    pub name: String,
    pub email: String,
    pub revision: u64,
    pub parent: Option<u64>,
    pub message: String,
    pub at: i64,
    pub git_commit: Option<String>,
    pub git_meta: Option<Vec<u8>>,
    /// Who the server let in for it (B7), when it recorded one.
    pub principal: Option<String>,
    pub chain: String,
}

/// A tree's entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaEntry {
    pub mode: String,
    pub raw_name: Vec<u8>,
    pub oid: String,
}

/// What an object is: a blob's bytes, or a tree's size and entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaContent {
    Blob(Vec<u8>),
    Tree {
        size: u64,
        entries: Vec<ReplicaEntry>,
    },
}

/// One object, at its place in the primary's order of objects (1, 2, ..,
/// with no gap), and its original git bytes when it was imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaObject {
    pub sequence: u64,
    pub oid: String,
    pub content: ReplicaContent,
    /// `Some` for an imported original (its raw tree, `None` for a blob).
    pub original: Option<Option<Vec<u8>>>,
}

/// An epoch: from which revision, why and when it was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaEpoch {
    pub epoch: u64,
    pub from_revision: u64,
    pub reason: String,
    pub at: i64,
}

/// The board's provenance row (its id, and an import's source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaMeta {
    pub board_id: Option<String>,
    pub algorithm: String,
    pub source_remote: Option<String>,
    pub source_ref: Option<String>,
    pub source_head: Option<String>,
    pub imported_count: u64,
}

/// An admin row: a token's name, date, kind and SHA-256 (never a token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminToken {
    pub name: Slug,
    pub created: Timestamp,
    pub kind: TokenKind,
    pub digest: String,
}

/// One batch of the primary's history after a follower's place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaBatch {
    /// The revision the batch follows, and the primary's chain there
    /// (none at 0, or when the primary has no such revision).
    pub after: u64,
    pub chain_after: Option<String>,
    /// The objects after the follower's, in order.
    pub objects: Vec<ReplicaObject>,
    /// The revisions after `after`, in order: only once every object the
    /// primary holds is in this batch or before it.
    pub revisions: Vec<ReplicaRevision>,
    pub epochs: Vec<ReplicaEpoch>,
    pub meta: Option<ReplicaMeta>,
    pub tokens: Vec<AdminToken>,
    /// The primary's newest revision, and whether more follow this batch.
    pub tip: u64,
    pub more: bool,
}

/// Where a follower is: its newest revision and that revision's chain
/// (none with no board), and how many objects it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaPosition {
    pub revision: u64,
    pub chain: Option<String>,
    pub objects: u64,
}

/// Why a batch was not applied: the store failed, the batch is not the
/// next one this store can take (it is malformed, or its rows hash to
/// another chain), or the primary's history is not this one's (a fork).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaError {
    Unavailable(String),
    Invalid(String),
    Diverged(String),
}

impl std::fmt::Display for ReplicaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(why) | Self::Invalid(why) | Self::Diverged(why) => f.write_str(why),
        }
    }
}

/// Port: the primary's history, as its store gives it to followers.
pub trait ReplicationLog: Send + Sync {
    /// The batch after revision `after` and object `objects_after`, at
    /// most `limit` revisions and about [`BATCH_BYTES`] of rows.
    ///
    /// # Errors
    /// The store could not be read.
    fn replica_batch(
        &self,
        after: u64,
        objects_after: u64,
        limit: BatchLimit,
    ) -> Result<ReplicaBatch, BoardStoreError>;
}

/// Port: the primary, as a follower reaches it: its batches (the
/// `replicate` route) and its change feed.
pub trait ReplicationSource: Send + Sync {
    /// As [`ReplicationLog::replica_batch`], from the primary.
    ///
    /// # Errors
    /// Why no batch came: busy, refused, or unreachable.
    fn batch(
        &self,
        after: u64,
        objects_after: u64,
        limit: BatchLimit,
    ) -> Result<ReplicaBatch, OpenRefused>;

    /// The primary's feed after `after` (none: from now).
    ///
    /// # Errors
    /// Why it did not open.
    fn open(&self, after: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused>;

    /// The primary's address, as a client names it (never a token).
    fn primary(&self) -> String;
}

/// Port: the follower's own board, which only batches change.
pub trait ReplicaStore: Send + Sync {
    /// Where it is.
    ///
    /// # Errors
    /// The store could not be read.
    fn position(&self) -> Result<ReplicaPosition, ReplicaError>;

    /// Applies `batch` whole or not at all: where it is then.
    ///
    /// # Errors
    /// As [`ReplicaError`]; nothing is applied.
    fn apply(&self, batch: &ReplicaBatch) -> Result<ReplicaPosition, ReplicaError>;

    /// Why this follower was found diverged, kept across restarts; none
    /// while it is not.
    ///
    /// # Errors
    /// The mark could not be read.
    fn diverged(&self) -> Result<Option<String>, ReplicaError>;

    /// Keeps that this follower diverged, and why, across restarts.
    ///
    /// # Errors
    /// The mark could not be kept.
    fn mark_diverged(&self, why: &str) -> Result<(), ReplicaError>;
}

/// A server's tokens as it starts to follow (B11): how many it holds, and
/// whether they are the set it last took from a primary, not tokens of its
/// own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowedTokens {
    pub held: usize,
    pub copied: bool,
}

/// Port: a follower's tokens, as following keeps them (B11).
pub trait FollowTokens: Send + Sync {
    /// The tokens held now.
    ///
    /// # Errors
    /// The store failed.
    fn followed(&self) -> Result<FollowedTokens, TokenError>;
}

/// Port: the server's admin rows (its device tokens' digests): a primary
/// gives them, a follower takes them in whole.
pub trait AdminRows: Send + Sync {
    /// The rows held, by name.
    ///
    /// # Errors
    /// The store failed.
    fn export(&self) -> Result<Vec<AdminToken>, TokenError>;

    /// Holds `rows` alone from now on (a token not among them is revoked).
    ///
    /// # Errors
    /// A row not as a token is kept, or the store failed.
    fn replace(&self, rows: &[AdminToken]) -> Result<(), TokenError>;
}

/// What a follower is doing: catching up, following the feed, stalled
/// (it tries again; why), or diverged (it refuses to go on; why).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaState {
    CatchingUp,
    Following,
    Stalled(String),
    Diverged(String),
}

impl ReplicaState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CatchingUp => "catching_up",
            Self::Following => "following",
            Self::Stalled(_) => "stalled",
            Self::Diverged(_) => "diverged",
        }
    }

    /// Why it is stalled or diverged.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::CatchingUp | Self::Following => None,
            Self::Stalled(why) | Self::Diverged(why) => Some(why),
        }
    }
}

/// A follower's status: its primary, the revision it applied last, the
/// primary's newest as last heard (none before the first batch), and what
/// it is doing. Its lag is the difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaStatus {
    pub primary: String,
    pub applied: u64,
    pub primary_head: Option<u64>,
    pub state: ReplicaState,
}
