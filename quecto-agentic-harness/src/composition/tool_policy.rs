//! Tool-policy composition (#1849, #2024): the durable persistence hook a
//! loop's `set_tool_policy … persist` writes through, over the run's
//! selected config file by way of the one configuration writer
//! (`infrastructure::config::writer::tool_policy`). `main` hands
//! [`build_tool_policy_persistence`] to the CLI entry point; the agent
//! build installs the hook on the loop it builds.

use std::path::Path;

use crate::application::agent_loop::ToolPolicyPersistence;
use crate::application::configuration::dto::ConfigSources;
use crate::infrastructure::config::writer::tool_policy::tool_policy_persistence_for;

/// The persistence hook that patches applied tool-policy mutations into
/// the run's base file as `tools.policy.entries`, the baseline a later
/// reload re-applies; the rest of the file is left as written. An entry
/// the *applied* overlay defines is refused rather than shadowed. The
/// write holds the document's lock under `base_dir`, the same one
/// `quecto config set` takes.
pub fn build_tool_policy_persistence(
    base_dir: &Path,
    sources: &ConfigSources,
) -> ToolPolicyPersistence {
    tool_policy_persistence_for(
        base_dir,
        sources.base.clone(),
        sources.applied_overlay().cloned(),
    )
}

#[cfg(test)]
#[path = "tool_policy_tests.rs"]
mod tests;
