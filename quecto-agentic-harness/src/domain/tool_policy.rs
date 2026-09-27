//! Tool-policy rules over the tool catalogue.

use std::collections::BTreeMap;

use super::tool_descriptor::{ProfileAvailabilityScope, ToolCatalogueEntry};
use super::tool_id::stable_tool_id;

/// The child policy a parent's catalogue implies: each tool's effective
/// scope keyed by stable id, plus its name when the id is the generated one
/// (a child with another owner derives a different id for a name-only tool).
/// The one fold every inherited-policy snapshot is built from (#2216).
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
        snapshot.insert(stable_id, scope);
        if is_generated_stable_id {
            snapshot.insert(name, scope);
        }
    }
    snapshot
}
