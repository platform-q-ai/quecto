//! Use cases of the sessions capability. Constructed only by
//! `composition::sessions`; interface holds injected handles.

pub mod clear_conversation;
pub mod departing_children;
pub mod export_session_report;
pub mod list_sessions;
pub mod read_history;
pub mod recall_context;
pub mod recover_message;
pub mod resume_saved_session;
pub mod retain_context;
pub mod rewind_conversation;
pub mod save_session;
pub mod search_session_metadata;
pub mod start_fresh_conversation;
pub mod synchronize_transcript;

pub use clear_conversation::ClearConversation;
pub use departing_children::DepartingChildren;
pub use export_session_report::ExportSessionReport;
pub use list_sessions::ListSessions;
pub use read_history::ReadHistory;
pub use recall_context::RecallContext;
pub use recover_message::RecoverMessage;
pub use resume_saved_session::ResumeSavedSession;
pub use retain_context::{ListRetainedContext, RetainContext};
pub use rewind_conversation::RewindConversation;
pub use save_session::SaveSession;
pub use search_session_metadata::SearchSessionMetadata;
pub use start_fresh_conversation::StartFreshConversation;
pub use synchronize_transcript::SynchronizeTranscript;

#[cfg(test)]
#[path = "conversation_rewrite_rig_tests.rs"]
pub(crate) mod conversation_rewrite_rig;
#[cfg(test)]
#[path = "retention_rig_tests.rs"]
pub(crate) mod retention_rig;
#[cfg(test)]
#[path = "start_fresh_rig_tests.rs"]
pub(crate) mod start_fresh_rig;
