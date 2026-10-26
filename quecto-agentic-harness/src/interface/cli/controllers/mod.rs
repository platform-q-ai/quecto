//! CLI controllers (#2371, L13): parse a command line, invoke the use case
//! or handle it names, and hand the result to the terminal.

pub(in crate::interface::cli) mod admission_broker;
pub mod admission_handles;
pub(super) mod agent;
pub mod auth;
pub mod board;
pub mod catalogue_handles;
pub mod claude_member;
pub(super) mod commands;
pub(super) mod config_cmd;
pub(super) mod config_flag;
pub(in crate::interface::cli) mod config_loading;
pub mod configuration_handles;
pub(super) mod container;
pub mod container_config_handles;
pub mod container_handles;
pub(super) mod container_inventory;
pub(super) mod container_setup;
pub(super) mod help;
pub(in crate::interface) mod launch_rollback_wait;
pub(super) mod models;
pub mod retention_handles;
pub mod run_end_fleet;
#[path = "agent/run_session_parent.rs"]
pub(crate) mod run_session_parent;
pub mod shared;
pub mod swarm_board_handles;
pub(in crate::interface) mod swarm_composition;
pub(crate) mod tool_runtime;

#[cfg(test)]
mod find_runtime_tests;
