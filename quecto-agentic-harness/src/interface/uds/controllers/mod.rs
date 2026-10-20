//! UDS controllers (#2369): the dispatch loop and the handlers that map one
//! wire request to one use case.
#[cfg(test)]
#[path = "subagent_teardown_ack_fakes_tests.rs"]
pub(crate) mod ack_fakes;
pub mod export_report_controller;
pub mod list_models_controller;
pub mod read_history_controller;
pub mod recover_message_controller;
pub mod resume_session_controller;
pub mod rewind_conversation_controller;
pub mod search_session_metadata_controller;
pub mod sessions_controller;
pub mod subagent_teardown_controller;
pub mod synchronize_transcript_controller;
pub mod uds;
pub(in crate::interface::uds) mod uds_busy_get_message;
pub(in crate::interface::uds) mod uds_busy_subagents;
#[cfg(test)]
mod uds_busy_subagents_tests;
pub mod uds_cancel;
mod uds_cancel_history;
pub(in crate::interface) mod uds_control_forward;
pub(in crate::interface) mod uds_delete_all_subagents;
pub mod uds_discovery_handles;
mod uds_dispatch_reload;
pub(in crate::interface) mod uds_ext_protocol;
pub(in crate::interface) mod uds_extensions;
pub mod uds_models;
pub mod uds_parent_control;
mod uds_progress_forward;
mod uds_query;
pub(in crate::interface) mod uds_reader_dispatch;
pub mod uds_session_handles;
pub(in crate::interface) mod uds_swarm_board;
pub(in crate::interface) mod uds_swarm_control;
pub(crate) mod uds_teardown_adapters;
pub mod uds_teardown_handles;
pub mod uds_turn_accounting;
mod uds_workflow_nudge;
