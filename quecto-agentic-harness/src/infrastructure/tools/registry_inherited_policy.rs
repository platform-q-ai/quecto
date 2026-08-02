use crate::infrastructure::tools::registration::ToolRegistration;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

impl ToolRegistryImpl {
    pub(crate) fn apply_inherited_tool_policy_snapshot(
        &mut self,
        snapshot: &crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
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
