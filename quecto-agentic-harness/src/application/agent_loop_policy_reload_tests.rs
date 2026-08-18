use super::super::tests::*;
use super::policy_tests::mock_catalogue_entry;
use super::*;
use crate::domain::tool::{
    RuntimeToolLifecycleRegistry, ToolPolicyMutator, ToolProfileContext, ToolRegistry,
};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

struct ReloadPolicyRegistry {
    entries: Vec<crate::domain::tool_descriptor::ToolCatalogueEntry>,
}

impl ToolCatalog for ReloadPolicyRegistry {
    fn definitions(&self) -> &[crate::domain::tool::ToolDefinition] {
        &[]
    }

    fn definitions_for(
        &self,
        _profile: ToolProfileContext,
    ) -> &[crate::domain::tool::ToolDefinition] {
        &[]
    }

    fn catalogue_entries(&self) -> Vec<crate::domain::tool_descriptor::ToolCatalogueEntry> {
        self.entries.clone()
    }
}

impl ToolExecutor for ReloadPolicyRegistry {
    fn execute(
        &self,
        _name: &str,
        _arguments: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::domain::tool::ToolResult, DomainError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move { Err(DomainError::Tool("not implemented".into())) })
    }
}

impl RuntimeToolLifecycleRegistry for ReloadPolicyRegistry {}
impl SessionAwareTools for ReloadPolicyRegistry {}
impl ToolPolicyMutator for ReloadPolicyRegistry {
    fn apply_persisted_tool_policy_entries(
        &mut self,
        entries: &std::collections::HashMap<String, ProfileAvailabilityScope>,
    ) -> Vec<String> {
        for entry in &mut self.entries {
            if let Some(scope) = entries.get(entry.name.as_ref()) {
                entry.profile_scope = Some(*scope);
                entry.profile_enabled = Some(scope.is_enabled());
                entry.effective_scope = *scope;
                entry.effective_parent_enabled = scope.allows_parent();
                entry.effective_child_enabled = scope.allows_child();
                entry.effective_enabled = scope.is_enabled();
            }
        }
        Vec::new()
    }
}
impl ToolRegistry for ReloadPolicyRegistry {}

#[test]
fn reload_persisted_policy_entries_clear_stale_runtime_overlay() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let registry = ReloadPolicyRegistry {
        entries: vec![mock_catalogue_entry("write", true)],
    };
    let mut agent = AgentLoopImpl::new(test_config(provider, Box::new(registry)));

    agent
        .tool_policy_state
        .lock()
        .unwrap()
        .record_applied("write", ProfileAvailabilityScope::None);
    assert_eq!(
        agent
            .tool_catalogue_entries()
            .into_iter()
            .find(|entry| entry.name.as_ref() == "write")
            .unwrap()
            .effective_scope,
        ProfileAvailabilityScope::None
    );

    let mut entries = std::collections::HashMap::new();
    entries.insert("write".to_string(), ProfileAvailabilityScope::Both);
    assert!(
        agent
            .apply_persisted_tool_policy_entries(&entries)
            .is_empty()
    );

    let entry = agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name.as_ref() == "write")
        .expect("write catalogue entry");
    assert_eq!(entry.effective_scope, ProfileAvailabilityScope::Both);
    assert!(entry.effective_parent_enabled);
    assert!(entry.effective_child_enabled);
}
