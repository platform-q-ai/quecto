//! Capability-local ports of the tools capability (#1960).
//!
//! The agent turn executes tools through these contracts; infrastructure
//! implements them (native tools, the registry, UDS-delivered tools) and
//! composition wires concrete instances. The pure tool vocabulary
//! (definitions, results, policy values) stays in `domain::tool`.
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{
    ToolDefinition, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationResult,
    ToolPolicyMutationStatus, ToolPolicyReconciliation, ToolPolicyRequest, ToolProfileContext,
    ToolResult,
};
use crate::domain::tool_descriptor::{
    ProfileAvailabilityScope, ToolCatalogueEntry, ToolDescriptor, ToolSource,
};

/// Port: a tool the agent can invoke.
///
/// Uses `Pin<Box<dyn Future>>` return type for dyn-compatibility,
/// allowing tools to be stored in a registry as `Arc<dyn Tool>`.
///
/// # Error handling contract
///
/// `execute` distinguishes **LLM-addressable** errors from **infrastructure**
/// errors via its return type:
///
/// - `Ok(ToolResult { is_error: true, content })` — the LLM supplied bad
///   input (malformed JSON arguments, missing or invalid field, forbidden
///   path, tool-specific validation failure). The agent loop surfaces
///   `content` back to the LLM so it can read the explanation and retry.
///   **This is the preferred form for anything the LLM can fix.**
///
/// - `Err(DomainError)` — an infrastructure-level failure the LLM cannot
///   reasonably correct (OS I/O error, sandbox violation, timeout, upstream
///   service outage). The agent loop still surfaces these as tool errors,
///   but adapter authors should reserve them for genuinely external failures.
///
/// Both forms terminate the tool call with `is_error: true` from the agent
/// loop's perspective, so neither is "wrong"; the distinction is a style
/// contract for readability and future telemetry, not a functional one.
/// Adapter unit tests should assert the chosen shape to pin down behaviour.
pub trait Tool: Send + Sync {
    /// Called after a non-error tool result has been appended to the parent conversation and run ledger.
    fn result_delivered(&self, _arguments: &str, _result: &ToolResult) {}
    /// Return the tool's definition for the LLM.
    fn definition(&self) -> ToolDefinition;

    /// Notify stateful tools that the active session key changed.
    fn set_session_key(&self, _session_key: String) {}

    /// Update a stateful spawn-like tool with the current child policy ceiling.
    ///
    /// Agent-loop callers can route through `set_inherited_child_policy_snapshot_for_spawn`
    /// on `RuntimeToolLifecycleRegistry` so the snapshot is derived from the
    /// authoritative `ToolPolicyState` overlay, not stale registry catalogue state.
    fn set_inherited_child_policy_snapshot_for_spawn(
        &self,
        _snapshot: BTreeMap<String, ProfileAvailabilityScope>,
    ) {
    }

    /// Inspect a stateful spawn-like tool's current child policy ceiling.
    fn inherited_child_policy_snapshot_for_spawn(
        &self,
    ) -> Option<BTreeMap<String, ProfileAvailabilityScope>> {
        None
    }

    /// Execute the tool with JSON-encoded arguments.
    ///
    /// See the trait-level docs for the error-handling contract:
    /// prefer `Ok(ToolResult { is_error: true })` for LLM-addressable
    /// errors; use `Err` only for infrastructure failures.
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>>;
}

/// A guard that can inspect and block tool calls before execution.
///
/// Guards run in registration order before every `execute()` call on the
/// tool registry.  The first `Err` short-circuits — remaining guards and
/// the tool itself are skipped.  The error string is returned to the LLM
/// as a `ToolResult { is_error: true }` so it can act on the feedback.
pub trait ToolGuard: Send + Sync {
    /// Inspect a tool call before execution.
    /// Return `Ok(())` to allow, `Err(reason)` to block.
    fn check(&self, tool_name: &str, arguments: &str) -> Result<(), String>;
}

/// Port: a read-only catalog of tools available to the model.
///
/// Use this role when callers only need schema/name visibility and should not
/// execute or mutate the registry.
pub trait ToolCatalog: Send + Sync {
    /// Return enabled tool definitions visible to the model.
    fn definitions(&self) -> &[ToolDefinition];

    /// Return enabled tool definitions visible to the requested profile context.
    fn definitions_for(&self, _context: ToolProfileContext) -> &[ToolDefinition] {
        self.definitions()
    }

    /// Return descriptors for all registered tools, including disabled tools
    /// hidden from model-visible definitions.
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.definitions()
            .iter()
            .cloned()
            .map(|definition| ToolDescriptor::enabled(definition, ToolSource::Runtime, "runtime"))
            .collect()
    }

    /// Return rich additive catalogue/effective-policy state when the backing
    /// catalogue has real metadata. The default fallback is intentionally empty:
    /// callers that need TUI/API-ready state must use a metadata-aware registry.
    fn catalogue_entries(&self) -> Vec<ToolCatalogueEntry> {
        Vec::new()
    }

    /// Return the number of registered tools without cloning definitions.
    ///
    /// Override if `definitions()` is not O(1) (e.g. lazy-building implementations).
    fn tool_count(&self) -> usize {
        self.definitions().len()
    }
}

/// Port: execution-only access to registered tools.
///
/// Uses `Pin<Box<dyn Future>>` for dyn-compatibility.
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool by name with JSON arguments.
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>>;

    /// Acknowledge that a non-error result was durably delivered to the caller's context.
    fn result_delivered(&self, _name: &str, _arguments: &str, _result: &ToolResult) {}
}

/// Port: live runtime policy mutation for registered tools.
pub trait ToolPolicyMutator: Send + Sync {
    fn record_persisted_tool_policy_results(&mut self, _reconciliation: &ToolPolicyReconciliation) {
    }

    fn apply_persisted_tool_policy_entries(
        &mut self,
        _entries: &HashMap<String, ProfileAvailabilityScope>,
    ) -> Vec<String> {
        Vec::new()
    }

    fn rollback_tool_policy_results(&mut self, reconciliation: &ToolPolicyReconciliation) {
        let mutations = reconciliation
            .results
            .iter()
            .filter(|result| {
                matches!(
                    result.status,
                    ToolPolicyMutationStatus::Applied | ToolPolicyMutationStatus::AlreadyInState
                )
            })
            .filter_map(|result| {
                let before = result.before.as_ref()?;
                Some(ToolPolicyMutation::set_scope(
                    result.name.clone(),
                    before.effective_scope,
                    "rollback failed persisted tool policy update",
                ))
            })
            .collect::<Vec<_>>();
        if !mutations.is_empty() {
            let _ = self.apply_tool_policy_mutations(&mutations, reconciliation.mode);
        }
    }

    fn apply_tool_policy_mutations(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        self.apply_tool_policy_request(&ToolPolicyRequest::patch(mutations.to_vec()), mode)
    }

    fn apply_tool_policy_request(
        &mut self,
        request: &ToolPolicyRequest,
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        ToolPolicyReconciliation {
            mode,
            results: request
                .mutations
                .iter()
                .map(|mutation| ToolPolicyMutationResult {
                    name: mutation.name.clone(),
                    requested_identifier: None,
                    requested_availability: mutation.availability,
                    requested_scope: mutation.scope,
                    status: ToolPolicyMutationStatus::UnknownTool,
                    before: None,
                    after: None,
                    reason: mutation.reason.clone(),
                })
                .collect(),
            correlation_id: request.correlation_id.clone(),
        }
    }
}

/// Port: runtime-loadable UDS tool lifecycle management.
///
/// This port is only for tools delivered by runtime lifecycle adapters such as
/// UDS. Bundled native provider tools are compiled into Quecto and remain in the
/// catalogue while policy enables/disables them; they must not use this unload
/// path.
pub trait RuntimeToolLifecycleRegistry: Send + Sync {
    /// Return names of currently unloadable runtime tools.
    fn runtime_tool_names(&self) -> Vec<String> {
        vec![]
    }

    /// Register a runtime-loadable tool.
    fn register_runtime_tool(&mut self, _tool: Arc<dyn Tool>) -> bool {
        false
    }

    /// Unregister a runtime-loadable tool by name.
    ///
    /// No-op if the name is not owned by a runtime provider. Bundled native tools
    /// are disabled/enabled by policy rather than unloaded. Default: no-op.
    fn unregister_runtime_tool(&mut self, _name: &str) {}

    /// Unregister runtime-loadable tools owned by one dynamic provider/client.
    fn unregister_runtime_tools_for_owner(&mut self, _owner: &str) -> Vec<String> {
        vec![]
    }

    /// Register a UDS-delivered runtime-loadable tool.
    fn register_uds_tool(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_runtime_tool(tool)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    fn extension_names(&self) -> Vec<String> {
        self.runtime_tool_names()
    }

    /// Compatibility name for the legacy extension lifecycle API.
    fn register_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_runtime_tool(tool)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    fn unregister_extension(&mut self, name: &str) {
        self.unregister_runtime_tool(name)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    fn unregister_extensions_for_owner(&mut self, owner: &str) -> Vec<String> {
        self.unregister_runtime_tools_for_owner(owner)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    fn register_uds_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_uds_tool(tool)
    }

    /// Return whether a UDS-delivered runtime-loadable tool with this name and
    /// owner would be accepted by the registry without mutating it.
    fn can_register_uds_tool_for_owner(&self, _name: &str, _owner: &str) -> bool {
        true
    }

    fn can_register_uds_tool_for_owner_with_stable_id(
        &self,
        name: &str,
        owner: &str,
        _stable_id: Option<&str>,
    ) -> bool {
        self.can_register_uds_tool_for_owner(name, owner)
    }

    /// Register a UDS-delivered runtime-loadable tool owned by a specific UDS
    /// client/connection.
    ///
    /// Default delegates to the ownerless UDS path for registries that do not
    /// expose per-connection ownership metadata.
    fn register_uds_tool_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        _owner: Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool(tool)
    }

    fn register_uds_tool_for_owner_with_stable_id(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
        _stable_id: Option<String>,
    ) -> bool {
        self.register_uds_tool_for_owner(tool, owner)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    fn can_register_uds_extension_for_owner(&self, name: &str, owner: &str) -> bool {
        self.can_register_uds_tool_for_owner(name, owner)
    }

    /// Update a stateful spawn-like tool with a child policy ceiling snapshot.
    fn set_inherited_child_policy_snapshot_for_spawn(
        &self,
        _snapshot: BTreeMap<String, ProfileAvailabilityScope>,
    ) {
    }

    #[cfg(test)]
    fn captured_spawn_snapshot(&self) -> Option<BTreeMap<String, ProfileAvailabilityScope>> {
        None
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    fn register_uds_extension_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool_for_owner(tool, owner)
    }

    /// Runtime-enable a registered tool. Default: unsupported.
    fn enable_tool(&mut self, _name: &str) -> bool {
        false
    }

    /// Runtime-disable a registered tool. Default: unsupported.
    fn disable_tool(&mut self, _name: &str) -> bool {
        false
    }
}

/// Port: session-key propagation for stateful tools.
pub trait SessionAwareTools: Send + Sync {
    /// Notify stateful tools that the active session key changed.
    fn set_session_key(&self, _session_key: &str) {}
}

/// Port: full tool registry capability used at composition boundaries.
///
/// Most application call sites should prefer one of the role-specific ports
/// above when they need only catalog, execution, extension lifecycle, or
/// session propagation behaviour. The full registry remains as the ergonomic
/// bundle owned by `AgentLoopImpl`.
pub trait ToolRegistry:
    ToolCatalog + ToolExecutor + RuntimeToolLifecycleRegistry + SessionAwareTools + ToolPolicyMutator
{
}

/// Dynamic execution admission, evaluated immediately before every tool effect.
pub trait ToolExecutionAdmission: Send + Sync {
    fn check<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>>;
}

#[cfg(test)]
#[path = "ports_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "ports_tests.rs"]
mod tests;
