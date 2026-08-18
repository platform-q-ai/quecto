use super::ToolRegistryImpl;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::config::ToolPolicyConfig;
use crate::infrastructure::tools::registration::ToolRegistration;

impl ToolRegistryImpl {
    pub(super) fn persisted_scope_for(
        &self,
        name: &str,
        metadata: &ToolRegistration,
    ) -> Option<ProfileAvailabilityScope> {
        let identity = metadata.identity_for_name(name);
        self.persisted_policy_scopes
            .get(identity.stable_id.as_ref())
            .copied()
            .or_else(|| self.persisted_policy_scopes.get(name).copied())
    }

    pub(super) fn apply_retained_persisted_policy(
        &self,
        name: &str,
        metadata: &mut ToolRegistration,
    ) {
        if let Some(scope) = self.persisted_scope_for(name, metadata) {
            metadata.configured_enabled = Some(scope.is_enabled());
            metadata.configured_scope = Some(scope);
            metadata.profile_scope = Some(scope);
            metadata.profile_enabled = Some(scope.is_enabled());
        }
    }

    pub fn apply_persisted_tool_policy(&mut self, policy: &ToolPolicyConfig) -> Vec<String> {
        let mut unknown = Vec::new();
        for (stable_id, entry) in &policy.entries {
            self.persisted_policy_scopes
                .insert(stable_id.clone(), entry.scope);
            match self.name_for_stable_id(stable_id) {
                Some(name) => {
                    let metadata = self
                        .metadata
                        .entry(name)
                        .or_insert_with(ToolRegistration::official_native);
                    metadata.configured_enabled = Some(entry.scope.is_enabled());
                    metadata.configured_scope = Some(entry.scope);
                    metadata.profile_scope = Some(entry.scope);
                    metadata.profile_enabled = Some(entry.scope.is_enabled());
                }
                None => unknown.push(stable_id.clone()),
            }
        }
        self.rebuild_definitions();
        unknown.sort();
        unknown
    }
}
