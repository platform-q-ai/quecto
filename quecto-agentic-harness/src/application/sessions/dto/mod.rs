//! Boundary DTOs of the sessions capability (#1970–#1978). Scope-neutral:
//! they name sessions by [`SessionIdentity`] and never by a filename or
//! path, and carry domain values (messages, ids, ranges, ledger positions)
//! — never a wire value or event.

use crate::domain::session_identity::SessionKeyPrefix;

pub mod clear_conversation;
pub mod history;
pub mod list_sessions;
pub mod message_recovery;
pub mod resume_decision;
pub mod resume_saved_session;
pub mod retained_context;
pub mod rewind_conversation;
pub mod save_session;
pub mod search_session_metadata;
pub mod session_report;
pub mod start_fresh_conversation;
pub mod startup_refusal;
pub mod sync;

pub use clear_conversation::{ClearConversationError, ClearedConversation};
pub use history::{HistoryError, HistoryPage, HistoryQuery};
pub use list_sessions::{ListSessionsRequest, ListSessionsResult, ListedSession, SessionListScope};
pub use message_recovery::{
    ContentSelector, RecoveredContent, RecoveryError, RecoveryRequest, Utf8Range,
};
pub use resume_decision::{
    ActionAvailability, ResumeActionCapabilities, ResumeActionOffer, ResumeDecision, ResumeIntent,
    ResumeOutcome, ResumeRequest,
};
pub use resume_saved_session::{
    ResumeSavedSessionError, ResumeTarget, SavedSessionResumed, StartupSessionOpened,
};
pub use retained_context::{RecallError, RecallOutcome, RecallQuery, Retained};
pub use rewind_conversation::{RewindConversationError, RewindRequest, RewoundConversation};
pub use save_session::{SaveMode, SaveOutcome, SaveSessionError, SaveTrigger};
pub use search_session_metadata::{
    QueryGeneration, SearchFreshness, SearchLimit, SearchSessionMetadataRequest,
    SearchSessionMetadataResult, SessionMetadataRow,
};
pub use session_report::{
    ExportManifest, ExportRecord, RawExportReceipt, ReportError, ReportPreview, ReportRecovery,
    SessionReport,
};
pub use start_fresh_conversation::{
    FleetSettled, FleetSettlementOutcome, FreshConversationStarted, SessionTransition,
    SessionTransitionRefused, StartFreshConversationError,
};
pub use startup_refusal::StartupRefusal;
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
