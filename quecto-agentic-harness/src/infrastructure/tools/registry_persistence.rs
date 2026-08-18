use super::ToolRegistryImpl;
use crate::infrastructure::config::ToolPolicyConfig;
use crate::infrastructure::tools::registration::ToolRegistration;

impl ToolRegistryImpl {
    pub fn apply_persisted_tool_policy(&mut self, policy: &ToolPolicyConfig) -> Vec<String> {
        let mut unknown = Vec::new();
        for (stable_id, entry) in &policy.entries {
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
