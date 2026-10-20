//! UDS controllers (#2369): the dispatch loop and the handlers that map one
//! wire request to one use case.
pub mod uds;
pub mod uds_cancel;
mod uds_cancel_history;
pub(in crate::interface) mod uds_control_forward;
pub(in crate::interface) mod uds_delete_all_subagents;
mod uds_dispatch_reload;
pub(in crate::interface) mod uds_ext_protocol;
pub(in crate::interface) mod uds_extensions;
pub mod uds_models;
pub mod uds_parent_control;
mod uds_progress_forward;
mod uds_query;
pub(in crate::interface) mod uds_reader_dispatch;
pub(in crate::interface) mod uds_swarm_control;
mod uds_workflow_nudge;
