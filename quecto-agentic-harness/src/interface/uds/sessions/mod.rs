//! UDS edge of the sessions capability (#1968): controllers that map a wire
//! command onto one application use case. Presentation of the results
//! stays with the CLI dispatch modules that own the wire field names.

pub mod controller;
pub mod export_report_controller;
pub mod read_history_controller;
pub mod recover_message_controller;
pub mod resume_session_controller;
pub mod rewind_conversation_controller;
pub mod synchronize_transcript_controller;
