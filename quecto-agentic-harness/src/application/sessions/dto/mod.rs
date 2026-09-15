//! Boundary DTOs of the sessions capability (#1970–#1976). Scope-neutral:
//! they name sessions by [`SessionIdentity`] and never by a filename or
//! path, and carry domain values (messages, ids, ranges, ledger positions)
//! — never a wire value or event.

use crate::domain::session_identity::SessionKeyPrefix;

pub mod clear_conversation;
pub mod history;
pub mod message_recovery;
pub mod resume_saved_session;
pub mod rewind_conversation;
pub mod save_session;
pub mod session_report;
pub mod start_fresh_conversation;
pub mod sync;

pub use clear_conversation::{ClearConversationError, ClearedConversation};
pub use history::{HistoryError, HistoryPage, HistoryQuery};
pub use message_recovery::{
    ContentSelector, RecoveredContent, RecoveryError, RecoveryRequest, Utf8Range,
};
pub use resume_saved_session::{
    ResumeSavedSessionError, ResumeTarget, SavedSessionResumed, StartupSessionOpened,
};
pub use rewind_conversation::{RewindConversationError, RewindRequest, RewoundConversation};
pub use save_session::{SaveMode, SaveOutcome, SaveSessionError, SaveTrigger};
pub use session_report::{
    ExportManifest, ExportRecord, RawExportReceipt, ReportError, ReportPreview, ReportRecovery,
    SessionReport,
};
pub use start_fresh_conversation::{
    FleetSettled, FleetSettlementOutcome, FreshConversationStarted, SessionTransition,
    SessionTransitionRefused, StartFreshConversationError,
};
pub use sync::{SyncRequest, TranscriptDelta, TranscriptReset, TranscriptSync};

/// Which saved sessions a list query covers (#1861).
///
/// Exactly the two selections the store ever offered: every session, or the
/// sessions whose identity starts with an existing key prefix (`chat-`,
/// `cli:`). The adapter may use the prefix to skip non-matching files before
/// reading them; the prefix itself is an identity prefix, never a filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionListQuery {
    All,
    ExistingKeyPrefix(SessionKeyPrefix),
}

impl SessionListQuery {
    /// The identity prefix this query narrows to, if any.
    pub fn key_prefix(&self) -> Option<&SessionKeyPrefix> {
        match self {
            Self::All => None,
            Self::ExistingKeyPrefix(prefix) => Some(prefix),
        }
    }
}

#[cfg(test)]
#[path = "dto_tests.rs"]
mod tests;
