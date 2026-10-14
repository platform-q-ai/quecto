use super::super::coordinator_launch::SpawnPermit;
use super::super::registration::ToolRegistration;
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

/// The board's spawn permit follows this registry (S4c): its policy over
/// `spawn`, as a parent's own call is judged, and its guards.
impl ToolRegistryImpl {
    /// `permit` follows this registry's policy and guards from now on.
    pub fn watch_spawn_permit(&mut self, permit: SpawnPermit) {
        self.spawn_permit = Some(permit);
        self.refresh_spawn_permit();
    }

    /// Sets the watched permit, if any, to this registry's policy and
    /// guards now.
    pub(super) fn refresh_spawn_permit(&self) {
        let Some(permit) = &self.spawn_permit else {
            return;
        };
        let allowed = self.tools.contains_key("spawn") && {
            let metadata = self
                .metadata
                .get("spawn")
                .cloned()
                .unwrap_or_else(ToolRegistration::official_native);
            Self::effective_scope(&metadata).allows_parent()
        };
        permit.set(allowed, &self.guards);
    }
}
