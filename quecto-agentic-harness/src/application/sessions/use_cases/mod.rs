//! Use cases of the sessions capability. Constructed only by
//! `composition::sessions`; interface holds injected handles.

pub mod export_session_report;
pub mod list_sessions;
pub mod read_history;
pub mod recover_message;
pub mod save_session;
pub mod synchronize_transcript;

pub use export_session_report::ExportSessionReport;
pub use list_sessions::ListSessions;
pub use read_history::ReadHistory;
pub use recover_message::RecoverMessage;
pub use save_session::SaveSession;
pub use synchronize_transcript::SynchronizeTranscript;
