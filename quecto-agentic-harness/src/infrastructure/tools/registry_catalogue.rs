use crate::domain::tool_descriptor::{
    ProfileAvailabilityScope, ToolCatalogueEntry, ToolHealth, ToolLifecycleKind,
};
use crate::infrastructure::tools::registration::ToolRegistration;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

impl ToolRegistryImpl {
    /// Return rich additive catalogue/effective-policy state for all registered tools.
    pub fn catalogue_entries(&self) -> Vec<ToolCatalogueEntry> {
        let mut entries: Vec<ToolCatalogueEntry> = self
            .tools
            .iter()
            .map(|(name, tool)| {
                let definition = tool.definition();
                let metadata = self
                    .metadata
                    .get(name)
                    .cloned()
                    .unwrap_or_else(ToolRegistration::official_native);
                let effective_scope = Self::effective_scope(&metadata);
                let effective_enabled = effective_scope != ProfileAvailabilityScope::None;
                ToolCatalogueEntry {
                    stable_id: metadata.identity_for_name(name).stable_id,
                    name: definition.name.clone(),
                    label: definition.name.clone(),
                    description: definition.description.clone(),
                    input_schema: definition.parameters_schema.clone(),
                    source: metadata.source,
                    owner: metadata.owner.clone(),
                    provider_id: metadata.provider_id.clone(),
                    version: None,
                    lifecycle: if metadata.unloadable {
                        ToolLifecycleKind::RuntimeLoadable
                    } else {
                        ToolLifecycleKind::Bundled
                    },
                    configurable: true,
                    default_enabled: metadata.default_enabled,
                    configured_enabled: metadata.configured_enabled,
                    profile_enabled: metadata
                        .profile_scope
                        .map(ProfileAvailabilityScope::is_enabled),
                    profile_scope: metadata.profile_scope,
                    session_enabled: metadata.session_enabled,
                    explicit_restriction: metadata.explicit_restriction,
                    runtime_availability: metadata.availability,
                    effective_enabled,
                    effective_scope,
                    effective_parent_enabled: effective_scope.allows_parent(),
                    effective_child_enabled: effective_scope.allows_child(),
                    health: if effective_enabled {
                        ToolHealth::Ok
                    } else {
                        ToolHealth::Disabled
                    },
                }
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }
}
