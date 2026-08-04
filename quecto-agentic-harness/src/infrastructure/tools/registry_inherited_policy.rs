use std::collections::BTreeMap;

use crate::domain::tool_id::stable_tool_id;
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
        for (policy_id, scope) in &snapshot.tools {
            let Some(name) = self.resolve_tool_policy_id(policy_id).ok().or_else(|| {
                self.tools
                    .contains_key(policy_id)
                    .then(|| policy_id.clone())
            }) else {
                warnings.push(policy_id.clone());
                continue;
            };
            let metadata = self
                .metadata
                .entry(name)
                .or_insert_with(ToolRegistration::official_native);
            metadata.inherited_scope = Some(*scope);
            metadata.profile_scope = Some(*scope);
            metadata.profile_enabled = Some(scope.is_enabled());
        }
        for (name, metadata) in self.metadata.iter_mut() {
            let identity = metadata.identity_for_name(name);
            if !snapshot.tools.contains_key(identity.stable_id.as_ref())
                && !snapshot.tools.contains_key(name)
            {
                metadata.inherited_scope =
                    Some(crate::domain::tool_descriptor::ProfileAvailabilityScope::None);
                metadata.profile_scope =
                    Some(crate::domain::tool_descriptor::ProfileAvailabilityScope::None);
                metadata.profile_enabled = Some(false);
            }
        }
        self.rebuild_definitions();
        self.refresh_spawn_inherited_child_policy_snapshot();
        warnings
    }

    pub(crate) fn inherited_child_policy_snapshot_tools(
        &self,
    ) -> BTreeMap<String, crate::domain::tool_descriptor::ProfileAvailabilityScope> {
        let mut snapshot = BTreeMap::new();
        for tool in self.catalogue_entries() {
            let name = tool.name.into_owned();
            let stable_id = tool.stable_id.into_owned();
            let scope = tool.effective_scope;
            let is_generated_legacy_stable_id =
                stable_id == stable_tool_id(tool.source, tool.provider_id.as_ref(), &name);
            snapshot.insert(stable_id, scope);
            if is_generated_legacy_stable_id {
                snapshot.insert(name, scope);
            }
        }
        snapshot
    }

    pub(crate) fn refresh_spawn_inherited_child_policy_snapshot(&self) {
        let snapshot = self.inherited_child_policy_snapshot_tools();
        if let Some(spawn) = self.tools.get("spawn") {
            spawn.set_inherited_child_policy_snapshot_for_spawn(snapshot);
        }
    }
}
