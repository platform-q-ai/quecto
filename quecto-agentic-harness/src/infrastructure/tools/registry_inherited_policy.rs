use crate::infrastructure::tools::registration::ToolRegistration;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

impl ToolRegistryImpl {
    pub(crate) fn apply_inherited_tool_policy_snapshot(
        &mut self,
        snapshot: &crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        self.inherited_policy_scopes = snapshot
            .tools
            .iter()
            .map(|(name, scope)| (name.clone(), *scope))
            .collect();
        self.inherited_policy_default_scope =
            Some(crate::domain::tool_descriptor::ProfileAvailabilityScope::None);
        for (name, scope) in &snapshot.tools {
            if !self.tools.contains_key(name) {
                warnings.push(name.clone());
                continue;
            }
            let metadata = self
                .metadata
                .entry(name.clone())
                .or_insert_with(ToolRegistration::official_native);
            metadata.inherited_scope = Some(*scope);
            metadata.profile_scope = Some(*scope);
            metadata.profile_enabled = Some(scope.is_enabled());
        }
        for (name, metadata) in self.metadata.iter_mut() {
            if !snapshot.tools.contains_key(name) {
                metadata.inherited_scope =
                    Some(crate::domain::tool_descriptor::ProfileAvailabilityScope::None);
                metadata.profile_scope =
                    Some(crate::domain::tool_descriptor::ProfileAvailabilityScope::None);
                metadata.profile_enabled = Some(false);
            }
        }
        self.rebuild_definitions();
        warnings
    }

    pub(crate) fn inherited_child_policy_snapshot(
        &self,
    ) -> crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot {
        let tools = self
            .catalogue_entries()
            .into_iter()
            .map(|tool| (tool.name.into_owned(), tool.effective_scope))
            .collect();
        crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot::new(tools)
    }
}
