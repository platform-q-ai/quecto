//! Use cases of the sessions capability. Constructed only by
//! `composition::sessions`; interface holds injected handles.

pub mod clear_conversation;
pub mod departing_children;
pub mod export_session_report;
pub mod list_scoped_sessions;
pub mod list_sessions;
pub mod read_history;
pub mod recall_context;
pub mod recover_message;
pub mod open_original_session;
pub mod fork_session_into_scope;
pub mod locate_session_home;
pub mod resume_disposition;
pub mod resume_saved_session;
pub mod retain_context;
pub mod rewind_conversation;
pub mod save_session;
pub mod start_fresh_conversation;
pub mod synchronize_transcript;

pub use clear_conversation::ClearConversation;
pub use departing_children::DepartingChildren;
pub use export_session_report::ExportSessionReport;
pub use list_scoped_sessions::ListScopedSessions;
pub use list_sessions::ListSessions;
pub use read_history::ReadHistory;
pub use recall_context::RecallContext;
pub use recover_message::RecoverMessage;
pub use fork_session_into_scope::ForkSessionIntoScope;
pub use locate_session_home::LocateSessionHome;
pub use open_original_session::OpenOriginalSession;
pub use resume_disposition::{assert_same_scope_or_refuse, DecideResumeDisposition};
pub use resume_saved_session::ResumeSavedSession;
pub use retain_context::{ListRetainedContext, RetainContext};
pub use rewind_conversation::RewindConversation;
pub use save_session::SaveSession;
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
