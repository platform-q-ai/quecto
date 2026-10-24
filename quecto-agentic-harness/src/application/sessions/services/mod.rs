//! The sessions capability's services (#2357): the application-owned state
//! of the loop's active session and its live-conversation read model, which
//! composition builds once per loop and the use cases operate on; the
//! session-home observations restore admission consults; and the transcript
//! paging rule the history and sync use cases share.

pub mod active_session;
pub mod conversation_ledger;
pub mod history_paging;
pub mod session_home;
