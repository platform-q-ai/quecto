//! Tool-policy composition (#1849): the durable persistence hook a loop's
//! `set_tool_policy … persist` writes through, over the run's config file.
//! `main` hands [`build_tool_policy_persistence`] to the CLI entry point;
//! the agent build installs the hook on the loop it builds.

use std::path::Path;

use crate::application::agent_loop::ToolPolicyPersistence;
use crate::infrastructure::tool_policy_persistence::tool_policy_persistence_for;

/// The persistence hook that writes applied tool-policy mutations into
/// `config_path` as `tools.policy.entries`, the baseline a later reload
/// re-applies.
pub fn build_tool_policy_persistence(config_path: &Path) -> ToolPolicyPersistence {
    tool_policy_persistence_for(config_path.to_path_buf())
}

#[cfg(test)]
#[path = "tool_policy_tests.rs"]
mod tests;
