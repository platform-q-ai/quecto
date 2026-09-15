//! Use cases of the sessions capability. Constructed only by
//! `composition::sessions`; interface holds injected handles.

pub mod list_sessions;
pub mod read_history;
pub mod recover_message;

pub use list_sessions::ListSessions;
pub use read_history::ReadHistory;
pub use recover_message::RecoverMessage;
