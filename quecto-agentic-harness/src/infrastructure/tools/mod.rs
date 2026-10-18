pub mod agents;
pub mod bash;
pub mod call_work;
pub mod command_match;
pub mod docs;
pub mod filesystem;
pub mod find_fd;
pub mod grep;
#[cfg(any(test, feature = "test-support"))]
pub mod panic_probe;
pub mod path_utils;
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
pub mod swarm;
pub mod truncate;
pub mod web_search;
pub mod workflow_tool;
