//! Tool-policy rules over the tool catalogue.

use std::collections::BTreeMap;

use super::tool_descriptor::{ProfileAvailabilityScope, ToolCatalogueEntry, ToolSource};
use super::tool_id::{parse_stable_tool_id, stable_tool_id};

/// The child policy a parent's catalogue implies: each tool's effective
/// scope keyed by stable id, plus its name when the id is the generated one
/// (a child with another owner derives a different id for a name-only tool).
/// A configured extension's tools also record the extension itself, under
/// its policy key, with the widest scope among them (#2446): a child's own
/// instance may name its tools differently. The one fold every
/// inherited-policy snapshot is built from (#2216).
pub fn inherited_child_policy_from_catalogue(
    entries: impl IntoIterator<Item = ToolCatalogueEntry>,
) -> BTreeMap<String, ProfileAvailabilityScope> {
    let mut snapshot = BTreeMap::new();
    for tool in entries {
        let name = tool.name.into_owned();
        let stable_id = tool.stable_id.into_owned();
        let scope = tool.effective_scope;
        let is_generated_stable_id =
            stable_id == stable_tool_id(tool.source, tool.provider_id.as_ref(), &name);
        if let Some(key) = configured_extension_key(&stable_id) {
            let recorded = snapshot
                .entry(key.to_string())
                .or_insert(ProfileAvailabilityScope::None);
            *recorded = ProfileAvailabilityScope::from_parent_child(
                recorded.allows_parent() || scope.allows_parent(),
                recorded.allows_child() || scope.allows_child(),
            );
        }
        snapshot.insert(stable_id, scope);
        if is_generated_stable_id {
            snapshot.insert(name, scope);
        }
    }
    snapshot
}

/// The provider id namespace of configured extensions' tools (#2446).
const CONFIGURED_EXTENSION_PROVIDER: &str = "uds:extension:";

/// The stable id of tool `tool` registered by configured extension
/// `extension`: the same in every agent that launches it, whichever
/// connection registers it.
pub fn configured_extension_tool_id(extension: &str, tool: &str) -> String {
    stable_tool_id(
        ToolSource::Uds,
        &format!("{CONFIGURED_EXTENSION_PROVIDER}{extension}"),
        tool,
    )
}

/// The policy key of the configured extension `stable_id` belongs to: its
/// provider id, which no tool name or stable id can equal. `None` for any
/// other tool.
pub fn configured_extension_key(stable_id: &str) -> Option<&str> {
    parse_stable_tool_id(stable_id)
        .filter(|id| {
            id.source == ToolSource::Uds
                && id.provider_id.starts_with(CONFIGURED_EXTENSION_PROVIDER)
        })
        .map(|id| id.provider_id)
}

/// Whether an inherited-policy key names something that registers after
/// start-up, so a child that has no such tool yet is not warned: a UDS or
/// runtime tool's stable id, or a configured extension's key.
pub fn registers_after_startup(policy_key: &str) -> bool {
    policy_key.starts_with(CONFIGURED_EXTENSION_PROVIDER)
        || parse_stable_tool_id(policy_key)
            .is_some_and(|id| matches!(id.source, ToolSource::Uds | ToolSource::Runtime))
}

#[cfg(test)]
#[path = "tool_policy_tests.rs"]
mod tests;
