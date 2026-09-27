//! The stable ids a persisted `tools.policy` entry may name (#2217).
//!
//! A policy entry is keyed by a tool's stable id. One this entrypoint
//! registers no tool for is kept and ignored, which is right for a bundled
//! tool another entrypoint builds and for a tool a release retired, but
//! hides a typo: a restriction that never applies. This catalogue tells the
//! three apart. It is the one list of bundled tools; a drift test builds
//! every entrypoint's registry and checks each bundled registration is here.

use super::tool_descriptor::ToolSource;
use super::tool_id::stable_tool_id;

/// Every bundled tool any entrypoint may build: `(provider id, name)`.
pub const BUNDLED_TOOLS: &[(&str, &str)] = &[
    ("quecto:official-tools", "bash"),
    ("quecto:official-tools", "docs"),
    ("quecto:official-tools", "edit"),
    ("quecto:official-tools", "find"),
    ("quecto:official-tools", "grep"),
    ("quecto:official-tools", "ls"),
    ("quecto:official-tools", "read"),
    ("quecto:official-tools", "swarm"),
    ("quecto:official-tools", "write"),
    ("quecto:session-tools", "recall"),
    ("quecto:agent-control", "agent_cmd"),
    ("quecto:agent-control", "spawn"),
    ("quecto:workflow", "workflow"),
    ("web", "web_fetch"),
    ("web", "web_search"),
];

/// Bundled tools a release removed: `(provider id, name, removed by)`.
/// A policy entry naming one is dead and safe to delete.
pub const RETIRED_TOOLS: &[(&str, &str, &str)] =
    &[("quecto:official-tools", "python_lab", "#1684")];

/// What a policy entry that matched no registered tool names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmatchedPolicyEntry {
    /// A bundled tool this entrypoint does not build: kept for the others.
    BundledElsewhere,
    /// A tool a release removed: the entry is dead.
    Retired { removed_by: &'static str },
    /// No bundled tool, current or retired: most likely a typo.
    Unknown,
}

/// The stable id a bundled tool registers under.
pub fn bundled_stable_id(provider_id: &str, name: &str) -> String {
    stable_tool_id(ToolSource::BundledNative, provider_id, name)
}

/// Classify a policy entry that matched no registered tool.
pub fn classify_unmatched_policy_entry(stable_id: &str) -> UnmatchedPolicyEntry {
    let bundled = BUNDLED_TOOLS
        .iter()
        .any(|(provider_id, name)| bundled_stable_id(provider_id, name) == stable_id);
    if bundled {
        return UnmatchedPolicyEntry::BundledElsewhere;
    }
    RETIRED_TOOLS
        .iter()
        .find(|(provider_id, name, _)| bundled_stable_id(provider_id, name) == stable_id)
        .map_or(UnmatchedPolicyEntry::Unknown, |(_, _, removed_by)| {
            UnmatchedPolicyEntry::Retired { removed_by }
        })
}

#[cfg(test)]
#[path = "tool_policy_catalogue_tests.rs"]
mod tests;
