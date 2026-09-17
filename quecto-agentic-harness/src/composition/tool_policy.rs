//! Tool-policy composition (#1849, #2024): the durable persistence hook a
//! loop's `set_tool_policy … persist` writes through, over the run's
//! selected config file by way of the one configuration writer
//! (`infrastructure::config::writer::tool_policy`). `main` hands
//! [`build_tool_policy_persistence`] to the CLI entry point; the agent
//! build installs the hook on the loop it builds.

use crate::application::agent_loop::ToolPolicyPersistence;
use crate::application::configuration::dto::ConfigSelection;
use crate::infrastructure::config::writer::tool_policy::tool_policy_persistence_for;

/// The persistence hook that patches applied tool-policy mutations into
/// the selection's base file as `tools.policy.entries`, the baseline a
/// later reload re-applies; the rest of the file is left as written. An
/// entry the selection's overlay defines is refused rather than shadowed.
pub fn build_tool_policy_persistence(selection: &ConfigSelection) -> ToolPolicyPersistence {
    tool_policy_persistence_for(
        selection.path().to_path_buf(),
        selection.overlay_path().map(std::path::Path::to_path_buf),
    )
}

#[cfg(test)]
#[path = "tool_policy_tests.rs"]
mod tests;
