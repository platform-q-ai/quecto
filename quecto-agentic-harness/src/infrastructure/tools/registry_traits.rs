use std::borrow::Cow;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::ToolRegistryImpl;
use crate::domain::error::DomainError;
use crate::domain::tool::{
    RuntimeToolLifecycleRegistry, SessionAwareTools, Tool, ToolCatalog, ToolDefinition,
    ToolExecutor, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyReconciliation,
    ToolPolicyRequest, ToolProfileContext, ToolResult,
};
use crate::domain::tool_descriptor::{
    ProfileAvailabilityScope, ToolCatalogueEntry, ToolDescriptor,
};

impl ToolCatalog for ToolRegistryImpl {
    fn definitions(&self) -> &[ToolDefinition] {
        self.definitions()
    }

    fn definitions_for(&self, context: ToolProfileContext) -> &[ToolDefinition] {
        self.definitions_for(context)
    }

    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.descriptors()
    }

    fn catalogue_entries(&self) -> Vec<ToolCatalogueEntry> {
        self.catalogue_entries()
    }
}

impl ToolExecutor for ToolRegistryImpl {
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let name = name.to_string();
        let arguments = arguments.to_string();
        Box::pin(async move { self.execute(&name, &arguments).await })
    }
}

impl RuntimeToolLifecycleRegistry for ToolRegistryImpl {
    fn runtime_tool_names(&self) -> Vec<String> {
        self.runtime_tool_names()
    }

    fn register_runtime_tool(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_runtime_tool(tool)
    }

    fn unregister_runtime_tool(&mut self, name: &str) {
        self.unregister_runtime_tool(name);
    }

    fn unregister_runtime_tools_for_owner(&mut self, owner: &str) -> Vec<String> {
        self.unregister_runtime_tools_for_owner(owner)
    }

    fn register_uds_tool(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_uds_tool(tool)
    }

    fn can_register_uds_tool_for_owner(&self, name: &str, owner: &str) -> bool {
        self.can_register_uds_tool_for_owner(name, owner)
    }

    fn can_register_uds_tool_for_owner_with_stable_id(
        &self,
        name: &str,
        owner: &str,
        stable_id: Option<&str>,
    ) -> bool {
        self.can_register_uds_tool_for_owner_with_stable_id(name, owner, stable_id)
    }

    fn register_uds_tool_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool_for_owner(tool, owner)
    }

    fn register_uds_tool_for_owner_with_stable_id(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
        stable_id: Option<String>,
    ) -> bool {
        self.register_uds_tool_for_owner_with_stable_id(tool, owner, stable_id)
    }

    fn enable_tool(&mut self, name: &str) -> bool {
        self.enable_tool(name)
    }

    fn disable_tool(&mut self, name: &str) -> bool {
        self.disable_tool(name)
    }

    fn set_inherited_child_policy_snapshot_for_spawn(
        &self,
        snapshot: BTreeMap<String, ProfileAvailabilityScope>,
    ) {
        if let Some(spawn) = self.tools.get("spawn") {
            spawn.set_inherited_child_policy_snapshot_for_spawn(snapshot);
        }
    }
}

impl crate::domain::tool::ToolPolicyMutator for ToolRegistryImpl {
    fn record_persisted_tool_policy_results(&mut self, reconciliation: &ToolPolicyReconciliation) {
        use crate::domain::tool::ToolPolicyMutationStatus;
        for result in &reconciliation.results {
            if !matches!(
                result.status,
                ToolPolicyMutationStatus::Applied | ToolPolicyMutationStatus::AlreadyInState
            ) {
                continue;
            }
            let Some(metadata) = self.metadata.get_mut(result.name.as_ref() as &str) else {
                continue;
            };
            metadata.configured_enabled = Some(result.requested_scope.is_enabled());
            metadata.configured_scope = Some(result.requested_scope);
            let stable_id = metadata
                .identity_for_name(result.name.as_ref())
                .stable_id
                .into_owned();
            self.persisted_policy_scopes
                .insert(stable_id, result.requested_scope);
        }
    }

    fn apply_tool_policy_mutations(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        self.apply_tool_policy_mutations(mutations, mode)
    }

    fn apply_tool_policy_request(
        &mut self,
        request: &ToolPolicyRequest,
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        self.apply_tool_policy_request(request, mode)
    }
}

impl SessionAwareTools for ToolRegistryImpl {
    fn set_session_key(&self, session_key: &str) {
        self.set_session_key(session_key);
    }
}

impl crate::domain::tool::ToolRegistry for ToolRegistryImpl {}
