//! Use cases of the sessions capability. Constructed only by
//! `composition::sessions`; interface holds injected handles.

pub mod clear_conversation;
pub mod export_session_report;
pub mod list_sessions;
pub mod read_history;
pub mod recover_message;
pub mod rewind_conversation;
pub mod save_session;
pub mod synchronize_transcript;

pub use clear_conversation::ClearConversation;
pub use export_session_report::ExportSessionReport;
pub use list_sessions::ListSessions;
pub use read_history::ReadHistory;
pub use recover_message::RecoverMessage;
pub use rewind_conversation::RewindConversation;
pub use save_session::SaveSession;
pub use synchronize_transcript::SynchronizeTranscript;

#[cfg(test)]
#[path = "conversation_rewrite_rig_tests.rs"]
pub(crate) mod conversation_rewrite_rig;
