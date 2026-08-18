use super::ToolRegistryImpl;
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolCatalogueEntry};

impl ToolRegistryImpl {
    pub(super) fn restriction_ceiling(
        &self,
        entry: &ToolCatalogueEntry,
    ) -> ProfileAvailabilityScope {
        let default = ProfileAvailabilityScope::from_enabled(entry.default_enabled);
        // Configured/persisted policy is a user preference, not a hard ceiling
        // for future policy mutations. It participates in effective_scope(),
        // but requests must be able to replace a durable narrow preference
        // without manual config edits.
        let session = entry
            .session_enabled
            .map(ProfileAvailabilityScope::from_enabled)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let inherited = self
            .metadata
            .get(entry.name.as_ref())
            .and_then(|m| m.inherited_scope)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let restriction = if entry.explicit_restriction.is_some() {
            ProfileAvailabilityScope::None
        } else {
            ProfileAvailabilityScope::Both
        };
        default
            .intersection(session)
            .intersection(inherited)
            .intersection(restriction)
    }
}
