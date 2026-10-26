pub mod agent_cmd;
pub(super) mod agent_cmd_container_listing;
pub mod agent_cmd_containers;
pub mod agent_cmd_ended;
pub(super) mod agent_cmd_parse;
pub(super) mod agent_cmd_report;
pub mod coordinator_launch;
pub mod delegated_roster;
pub mod environment_commands;
pub mod environment_member_shutdown;
pub mod harness_lifecycle;
pub(crate) mod inherited_tool_policy;
#[cfg(test)]
#[path = "inherited_tool_policy_unit_tests.rs"]
mod inherited_tool_policy_unit_tests;
pub mod kept_environments;
pub mod launch_rollbacks;
pub mod owned_work;
pub mod owner_exit;
pub(super) mod process_tree;
pub mod retained_environment_teardown;
pub mod spawn;
pub(super) mod spawn_binary;
pub(super) mod spawn_container;
pub mod spawn_discovery;
pub(super) mod spawn_entry;
pub(super) mod spawn_inherited_policy;
pub(super) mod spawn_input;
pub(super) mod spawn_launch_args;
pub(super) mod spawn_launch_ports;
pub(super) mod spawn_lifecycle;
pub(super) mod spawn_proxy_bridge;
pub mod spawn_reaper;
pub(super) mod spawn_registry;
pub mod subagent_cascade;
pub(super) mod subagent_cleanup;
#[cfg(test)]
mod subagent_cleanup_tests;
pub mod subagent_compact_roster;
pub mod subagent_environment_wire;
pub mod subagent_identity;
pub(super) mod subagent_lifecycle;
pub mod subagent_monitor;
pub(super) mod subagent_monitor_canonical;
pub mod subagent_monitor_merge;
pub(super) mod subagent_monitor_registry;
pub(super) mod subagent_monitor_stall;
pub(super) mod subagent_monitor_truncate;
pub mod subagent_registry;
pub(crate) mod subagent_routing;
#[cfg(test)]
mod subagent_routing_tests;
#[cfg(test)]
#[path = "subagent_run_end_tests.rs"]
mod subagent_run_end_tests;
pub(super) mod subagent_status;
#[cfg(test)]
#[path = "subagent_status_tests.rs"]
mod subagent_status_tests;
pub mod subagent_teardown_registry;
pub mod subagent_teardown_wiring;
