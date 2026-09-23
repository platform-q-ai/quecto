pub mod agent_cmd;
pub mod agent_cmd_containers;
mod agent_cmd_parse;
mod agent_cmd_report;
pub mod bash;
pub mod command_match;
pub mod delegated_roster;
pub mod docs;
pub mod environment_commands;
pub mod environment_member_shutdown;
pub mod filesystem;
pub mod find_fd;
pub mod grep;
pub mod harness_lifecycle;
pub(crate) mod inherited_tool_policy;
#[cfg(test)]
#[path = "inherited_tool_policy_unit_tests.rs"]
mod inherited_tool_policy_unit_tests;
pub mod owner_exit;
pub mod path_utils;
mod process_tree;
pub mod recall;
pub mod registration;
#[cfg(test)]
mod registration_tests;
pub mod registry;
mod registry_catalogue;
mod registry_inherited_policy;
mod registry_lifecycle_compat;
mod registry_tool_ids;
mod registry_uds;
pub mod retained_environment_teardown;
pub mod spawn;
mod spawn_binary;
mod spawn_container;
pub mod spawn_discovery;
mod spawn_entry;
mod spawn_inherited_policy;
mod spawn_input;
mod spawn_launch_args;
mod spawn_launch_ports;
mod spawn_lifecycle;
mod spawn_proxy_bridge;
pub mod spawn_reaper;
mod spawn_registry;
pub mod subagent_cascade;
mod subagent_cleanup;
#[cfg(test)]
mod subagent_cleanup_tests;
pub mod subagent_compact_roster;
pub mod subagent_environment_wire;
pub mod subagent_identity;
mod subagent_lifecycle;
pub mod subagent_monitor;
mod subagent_monitor_canonical;
pub mod subagent_monitor_merge;
mod subagent_monitor_registry;
mod subagent_monitor_stall;
mod subagent_monitor_truncate;
pub mod subagent_registry;
pub(crate) mod subagent_routing;
#[cfg(test)]
mod subagent_routing_tests;
mod subagent_status;
pub mod subagent_teardown_registry;
pub mod subagent_teardown_wiring;
pub mod swarm;
#[cfg(test)]
mod swarm_ac_gap_tests;
mod swarm_admission;
mod swarm_board_worker;
pub mod swarm_bridge;
mod swarm_config;
pub mod swarm_control;
#[cfg(test)]
mod swarm_job_tests;
pub mod swarm_lifecycle;
pub mod swarm_member_termination;
mod swarm_output;
#[cfg(any(test, feature = "test-support"))]
pub mod swarm_test_support;
#[cfg(test)]
mod swarm_tests;
pub mod truncate;
pub mod web_search;
pub mod workflow_tool;

#[cfg(test)]
#[path = "subagent_status_tests.rs"]
mod subagent_status_tests;

#[cfg(test)]
mod swarm_bridge_tests;
#[cfg(test)]
mod swarm_scope_tests;
