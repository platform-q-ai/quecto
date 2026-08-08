// ToolRegistry: holds all Tool implementations, provides lookup by name.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

pub use super::registration::ToolRegistration;
use crate::domain::error::DomainError;
use crate::domain::tool::{
    Tool, ToolDefinition, ToolGuard, ToolPolicyApplyMode, ToolPolicyMutation,
    ToolPolicyMutationResult, ToolPolicyMutationStatus, ToolPolicyOperation,
    ToolPolicyReconciliation, ToolPolicyRequest, ToolProfileContext, ToolResult,
};
use crate::domain::tool_descriptor::{
    ProfileAvailabilityScope, ToolAvailability, ToolCatalogueEntry, ToolDescriptor,
    ToolRestrictionReason,
};
use crate::domain::tool_id::{ToolIdResolveError, equivalent_policy_inputs};
use crate::infrastructure::config::Config;

/// Registry of all available tools, keyed by name.
pub struct ToolRegistryImpl {
    pub(super) tools: HashMap<String, Arc<dyn Tool>>,
    pub(super) metadata: HashMap<String, ToolRegistration>,
    definitions: Vec<ToolDefinition>,
    parent_definitions: Vec<ToolDefinition>,
    child_definitions: Vec<ToolDefinition>,
    execution_profile_context: Option<ToolProfileContext>,
    guards: Vec<Arc<dyn ToolGuard>>,
    /// Names explicitly reserved away from future registration.
    ///
    /// Destructive `remove()` / `remove_all()` add names here while unregistering
    /// their descriptors. Startup policy restrictions add names here without
    /// unregistering existing tools, so disabled names remain described but UDS
    /// and other runtime registration paths cannot reintroduce or shadow them.
    pub(super) denied_names: std::collections::HashSet<String>,
    pub(super) denied_policy_ids: std::collections::HashSet<String>,
    pub(super) inherited_policy_scopes: HashMap<String, ProfileAvailabilityScope>,
    pub(super) inherited_policy_default_scope: Option<ProfileAvailabilityScope>,
}

impl std::fmt::Debug for ToolRegistryImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistryImpl")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Default for ToolRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistryImpl {
    /// The exec tool's max captured-output size, read from config in one place.
    pub fn exec_registry_settings_from_config(config: &Config) -> usize {
        config.agents.defaults.exec_max_capture_bytes
    }

    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            metadata: HashMap::new(),
            definitions: Vec::new(),
            parent_definitions: Vec::new(),
            child_definitions: Vec::new(),
            execution_profile_context: None,
            guards: Vec::new(),
            denied_names: std::collections::HashSet::new(),
            denied_policy_ids: std::collections::HashSet::new(),
            inherited_policy_scopes: HashMap::new(),
            inherited_policy_default_scope: None,
        }
    }

    pub fn set_execution_profile_context(&mut self, context: ToolProfileContext) {
        self.execution_profile_context = Some(context);
    }

    /// Register a guard that runs before every tool execution.
    ///
    /// Guards run in registration order. The first `Err` short-circuits.
    pub fn register_guard(&mut self, guard: Arc<dyn ToolGuard>) {
        self.guards.push(guard);
    }

    /// Return the number of registered guards.
    pub fn guard_count(&self) -> usize {
        self.guards.len()
    }

    /// Remove a tool by name and permanently block re-registration.
    ///
    /// Returns `true` if the tool was found and removed, `false` otherwise.
    /// The name is added to the denylist so bundled native and runtime-loadable
    /// registration paths will reject it.
    pub fn remove(&mut self, name: &str) -> bool {
        self.reserve_removed_tool_identity(name);
        if self.tools.remove(name).is_some() {
            self.metadata.remove(name);
            self.rebuild_definitions();
            true
        } else {
            false
        }
    }

    /// Remove multiple tools in one call (single `rebuild_definitions`).
    pub fn remove_all(&mut self, names: &[String]) -> Vec<String> {
        let mut warnings = Vec::new();
        for name in names {
            self.reserve_removed_tool_identity(name);
            if self.tools.remove(name.as_str()).is_none() {
                warnings.push(name.clone());
            }
            self.metadata.remove(name.as_str());
        }
        if warnings.len() < names.len() {
            // At least one tool was actually removed
            self.rebuild_definitions();
        }
        warnings
    }

    /// Reserve a tool name so future bundled-native or runtime registrations
    /// cannot introduce or shadow it.
    ///
    /// This is intentionally non-destructive: if a tool with this name already
    /// exists, its descriptor and concrete implementation remain registered.
    fn deny_registration_name(&mut self, name: &str) {
        self.denied_names.insert(name.to_string());
    }

    fn deny_policy_id(&mut self, policy_id: &str) {
        self.denied_policy_ids
            .extend(equivalent_policy_inputs(policy_id));
    }

    /// Apply startup `--disable-tool` policy.
    ///
    /// Existing tools are disabled but remain registered/described. Every named
    /// tool, including unknown names, is also denied for future registration so
    /// UDS clients cannot reintroduce a process-disabled capability later.
    /// Returns unknown names for caller-visible warnings.
    pub fn apply_startup_tool_restrictions(&mut self, names: &[String]) -> Vec<String> {
        self.apply_tool_restrictions(names, ToolRestrictionReason::ExplicitDisable)
    }

    /// Apply inherited spawn/read-only restrictions while keeping provenance
    /// distinct from top-level startup disables.
    pub fn apply_spawn_tool_restrictions(&mut self, names: &[String]) -> Vec<String> {
        self.apply_tool_restrictions(names, ToolRestrictionReason::Spawn)
    }

    fn apply_tool_restrictions(
        &mut self,
        names: &[String],
        reason: ToolRestrictionReason,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut rebuild_needed = false;
        for policy_id in names {
            let name = match self.resolve_tool_policy_id(policy_id) {
                Ok(name) => name,
                Err(_) => {
                    self.deny_registration_name(policy_id);
                    self.deny_policy_id(policy_id);
                    warnings.push(policy_id.clone());
                    continue;
                }
            };
            self.reserve_removed_tool_identity(&name);
            self.deny_policy_id(policy_id);
            if !self.tools.contains_key(&name) {
                warnings.push(policy_id.clone());
                continue;
            }
            let metadata = self
                .metadata
                .entry(name.clone())
                .or_insert_with(ToolRegistration::official_native);
            if metadata.availability != ToolAvailability::Disabled {
                metadata.availability = ToolAvailability::Disabled;
                rebuild_needed = true;
            }
            metadata.session_enabled = Some(false);
            metadata.explicit_restriction = Some(reason);
        }
        if rebuild_needed {
            self.rebuild_definitions();
        }
        warnings
    }

    /// Register a tool with explicit source/owner/lifecycle metadata.
    ///
    /// This is the common registration seam for bundled native providers and UDS
    /// proxies. Compatibility wrappers below preserve the older public API while
    /// routing through this single path.
    pub fn register_with_metadata(
        &mut self,
        tool: Arc<dyn Tool>,
        metadata: ToolRegistration,
    ) -> bool {
        let def = tool.definition();
        let name = def.name.clone().into_owned();
        if self.denied_names.contains(&name) {
            tracing::warn!(tool = %name, "register rejected: tool is on the denylist");
            return false;
        }
        if let Some(existing) = self.metadata.get(&name) {
            if !existing.unloadable {
                tracing::warn!(tool = %name, "register rejected: shadows non-unloadable tool");
                return false;
            }
            if existing.owner != metadata.owner {
                tracing::warn!(tool = %name, existing_owner = %existing.owner, new_owner = %metadata.owner, "register rejected: shadows another owner");
                return false;
            }
        }
        let mut metadata = metadata;
        match self.registration_identity_is_available(&name, &metadata) {
            Ok(()) => {}
            Err(ToolIdResolveError::Duplicate(id)) => {
                tracing::warn!(tool = %name, duplicate_id = %id, "register rejected: duplicate stable tool id or alias");
                return false;
            }
            Err(ToolIdResolveError::Unknown(_)) => unreachable!("register does not resolve ids"),
        }
        if let Some(scope) = self
            .inherited_scope_for(&name, &metadata)
            .or(self.inherited_policy_default_scope)
        {
            metadata.inherited_scope = Some(scope);
            metadata.profile_scope = Some(scope);
            metadata.profile_enabled = Some(scope.is_enabled());
        }
        self.metadata.insert(name.clone(), metadata);
        self.tools.insert(name, tool);
        self.rebuild_definitions();
        true
    }

    /// Register a bundled native official tool. No-op if the name is denied.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_with_metadata(tool, ToolRegistration::official_native())
    }

    /// Register a runtime-loadable extension tool.
    ///
    /// Kept as a compatibility API for UDS/runtime lifecycle callers and tests.
    /// Bundled native extension tools must not use this path; they should use
    /// `register`/`register_with_metadata` with `ToolSource::BundledNative` and
    /// `unloadable: false`.
    pub fn register_runtime_tool(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_with_metadata(tool, ToolRegistration::runtime("runtime:extension"))
    }

    /// Register a UDS-delivered extension tool.
    pub fn register_uds_tool(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_with_metadata(tool, ToolRegistration::uds())
    }

    /// Remove an unloadable tool by name.
    ///
    /// No-op if `name` is not unloadable, preventing runtime lifecycle events
    /// from removing bundled native official tools.
    pub fn unregister_runtime_tool(&mut self, name: &str) {
        if !self
            .metadata
            .get(name)
            .map(|metadata| metadata.unloadable)
            .unwrap_or(false)
        {
            return;
        }
        self.metadata.remove(name);
        self.tools.remove(name);
        self.rebuild_definitions();
    }

    /// Remove unloadable runtime tools owned by a delivery/lifecycle adapter.
    ///
    /// This keeps UDS disconnect cleanup scoped to the disconnecting client while
    /// preserving bundled native tools and other clients' runtime tools.
    pub fn unregister_runtime_tools_for_owner(&mut self, owner: &str) -> Vec<String> {
        let names: Vec<String> = self
            .metadata
            .iter()
            .filter(|(_, metadata)| metadata.unloadable && metadata.owner.as_ref() == owner)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &names {
            self.metadata.remove(name);
            self.tools.remove(name);
        }
        if !names.is_empty() {
            self.rebuild_definitions();
        }
        names
    }

    /// Return the names of currently unloadable runtime tools.
    pub fn runtime_tool_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .metadata
            .iter()
            .filter(|(_, metadata)| metadata.unloadable)
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    /// Compatibility name for the legacy extension lifecycle API.
    pub fn extension_names(&self) -> Vec<String> {
        self.runtime_tool_names()
    }

    /// Compatibility name for the legacy extension lifecycle API.
    pub fn register_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_runtime_tool(tool)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    pub fn register_uds_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_uds_tool(tool)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    pub fn can_register_uds_extension_for_owner(&self, name: &str, owner: &str) -> bool {
        self.can_register_uds_tool_for_owner(name, owner)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    pub fn register_uds_extension_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool_for_owner(tool, owner)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    pub fn unregister_extension(&mut self, name: &str) {
        self.unregister_runtime_tool(name)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    pub fn unregister_extensions_for_owner(&mut self, owner: &str) -> Vec<String> {
        self.unregister_runtime_tools_for_owner(owner)
    }

    /// Return descriptors for all registered tools.
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        let mut descriptors: Vec<ToolDescriptor> = self
            .tools
            .iter()
            .map(|(name, tool)| {
                let metadata = self
                    .metadata
                    .get(name)
                    .cloned()
                    .unwrap_or_else(ToolRegistration::official_native);
                ToolDescriptor::new(
                    tool.definition(),
                    metadata.source,
                    metadata.owner,
                    metadata.availability,
                )
            })
            .collect();
        descriptors.sort_by(|a, b| a.definition.name.cmp(&b.definition.name));
        descriptors
    }

    /// Return the descriptor for a registered tool.
    pub fn descriptor(&self, name: &str) -> Option<ToolDescriptor> {
        let tool = self.tools.get(name)?;
        let metadata = self
            .metadata
            .get(name)
            .cloned()
            .unwrap_or_else(ToolRegistration::official_native);
        Some(ToolDescriptor::new(
            tool.definition(),
            metadata.source,
            metadata.owner,
            metadata.availability,
        ))
    }

    fn catalogue_entry(&self, name: &str) -> Option<ToolCatalogueEntry> {
        self.catalogue_entries()
            .into_iter()
            .find(|entry| entry.name.as_ref() == name || entry.stable_id.as_ref() == name)
    }

    pub(super) fn effective_scope(metadata: &ToolRegistration) -> ProfileAvailabilityScope {
        let runtime = ProfileAvailabilityScope::from_enabled(metadata.availability.is_enabled());
        let default = ProfileAvailabilityScope::from_enabled(metadata.default_enabled);
        let configured = metadata
            .configured_enabled
            .map(ProfileAvailabilityScope::from_enabled)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let session = metadata
            .session_enabled
            .map(ProfileAvailabilityScope::from_enabled)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let inherited = metadata
            .inherited_scope
            .unwrap_or(ProfileAvailabilityScope::Both);
        let profile = metadata
            .profile_scope
            .unwrap_or(ProfileAvailabilityScope::Both);
        runtime
            .intersection(default)
            .intersection(configured)
            .intersection(session)
            .intersection(inherited)
            .intersection(profile)
    }

    fn restriction_ceiling(&self, entry: &ToolCatalogueEntry) -> ProfileAvailabilityScope {
        let default = ProfileAvailabilityScope::from_enabled(entry.default_enabled);
        let configured = entry
            .configured_enabled
            .map(ProfileAvailabilityScope::from_enabled)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let session = entry
            .session_enabled
            .map(ProfileAvailabilityScope::from_enabled)
            .unwrap_or(ProfileAvailabilityScope::Both);
        let restriction = if entry.explicit_restriction.is_some() {
            ProfileAvailabilityScope::None
        } else {
            ProfileAvailabilityScope::Both
        };
        let inherited = self
            .metadata
            .get(entry.name.as_ref())
            .and_then(|m| m.inherited_scope)
            .unwrap_or(ProfileAvailabilityScope::Both);
        default
            .intersection(configured)
            .intersection(session)
            .intersection(inherited)
            .intersection(restriction)
    }

    /// Apply live runtime policy mutations and return before/after snapshots.
    pub fn apply_tool_policy_mutations(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        self.apply_tool_policy_request(&ToolPolicyRequest::patch(mutations.to_vec()), mode)
    }

    pub fn apply_tool_policy_request(
        &mut self,
        request: &ToolPolicyRequest,
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        let mut requested = request.mutations.clone();
        if request.operation == ToolPolicyOperation::Replace {
            let unlisted_scope = request
                .unlisted_scope
                .unwrap_or(ProfileAvailabilityScope::None);
            let mut listed = std::collections::HashSet::new();
            for mutation in &requested {
                let resolved = self
                    .resolve_tool_policy_id(&mutation.name)
                    .unwrap_or_else(|_| mutation.name.clone());
                listed.insert(resolved);
            }
            for name in self.tools.keys() {
                if !listed.contains(name) {
                    requested.push(ToolPolicyMutation::set_scope(
                        name.clone(),
                        unlisted_scope,
                        "set_tool_policy replace unlisted",
                    ));
                }
            }
        }
        let mut results = Vec::with_capacity(requested.len());
        for mutation in &requested {
            let resolved_name = self
                .resolve_tool_policy_id(&mutation.name)
                .unwrap_or_else(|_| mutation.name.clone());
            let before = self.catalogue_entry(&resolved_name);
            let status = match before.as_ref() {
                None => ToolPolicyMutationStatus::UnknownTool,
                Some(entry) if !mutation.scope.is_subset_of(self.restriction_ceiling(entry)) => {
                    ToolPolicyMutationStatus::BlockedByRestriction
                }
                Some(entry)
                    if entry
                        .profile_scope
                        .unwrap_or(ProfileAvailabilityScope::Both)
                        == mutation.scope =>
                {
                    ToolPolicyMutationStatus::AlreadyInState
                }
                Some(_) => {
                    self.set_profile_scope(&resolved_name, mutation.scope);
                    ToolPolicyMutationStatus::Applied
                }
            };
            let after = self.catalogue_entry(&resolved_name);
            let requested_identifier = (mutation.name != resolved_name
                || status == ToolPolicyMutationStatus::UnknownTool)
                .then(|| mutation.name.clone());
            results.push(ToolPolicyMutationResult {
                name: resolved_name.clone(),
                requested_identifier,
                requested_availability: mutation.availability,
                requested_scope: mutation.scope,
                status,
                before,
                after,
                reason: mutation.reason.clone(),
            });
        }
        self.refresh_spawn_inherited_child_policy_snapshot();
        ToolPolicyReconciliation { mode, results }
    }

    /// Runtime-disable a registered tool without removing its descriptor.
    pub fn disable_tool(&mut self, name: &str) -> bool {
        self.set_availability(name, ToolAvailability::Disabled)
    }

    /// Mark a registered tool disabled by entrypoint defaults.
    pub fn disable_tool_by_entrypoint_default(&mut self, name: &str) -> bool {
        self.set_registration_metadata(name, |metadata| {
            *metadata = metadata.clone().with_entrypoint_default_enabled(false);
        })
    }

    /// Mark a registered tool disabled by inherited spawn policy.
    pub fn disable_tool_by_spawn_restriction(&mut self, name: &str) -> bool {
        self.set_registration_metadata(name, |metadata| {
            *metadata = metadata.clone().with_spawn_restriction();
        })
    }

    /// Runtime-enable a registered tool without restart.
    pub fn enable_tool(&mut self, name: &str) -> bool {
        self.set_registration_metadata(name, |metadata| {
            metadata.profile_scope = Some(ProfileAvailabilityScope::Both);
            metadata.profile_enabled = Some(true);
            if metadata.explicit_restriction.is_none() && metadata.session_enabled != Some(false) {
                metadata.availability = ToolAvailability::Enabled;
            }
        })
    }

    fn set_registration_metadata(
        &mut self,
        name: &str,
        update: impl FnOnce(&mut ToolRegistration),
    ) -> bool {
        if !self.tools.contains_key(name) {
            return false;
        }
        let metadata = self
            .metadata
            .entry(name.to_string())
            .or_insert_with(ToolRegistration::official_native);
        update(metadata);
        self.rebuild_definitions();
        true
    }

    fn set_profile_scope(&mut self, name: &str, scope: ProfileAvailabilityScope) -> bool {
        if !self.tools.contains_key(name) {
            return false;
        }
        let metadata = self
            .metadata
            .entry(name.to_string())
            .or_insert_with(ToolRegistration::official_native);
        if metadata.profile_scope == Some(scope) {
            return true;
        }
        metadata.profile_scope = Some(scope);
        metadata.profile_enabled = Some(scope != ProfileAvailabilityScope::None);
        metadata.availability = ToolAvailability::from(scope);
        self.rebuild_definitions();
        true
    }

    fn set_availability(&mut self, name: &str, availability: ToolAvailability) -> bool {
        let scope = match availability {
            ToolAvailability::Enabled => ProfileAvailabilityScope::Both,
            ToolAvailability::Disabled => ProfileAvailabilityScope::None,
        };
        self.set_profile_scope(name, scope)
    }

    /// Rebuild the cached definitions list from enabled registered tools.
    ///
    /// Deduplication is unnecessary: `self.tools` is a `HashMap<String, _>`
    /// keyed by `tool.definition().name`, so keys are inherently unique.
    pub(super) fn rebuild_definitions(&mut self) {
        let mut parent = Vec::new();
        let mut child = Vec::new();
        for (name, tool) in &self.tools {
            let metadata = self
                .metadata
                .get(name)
                .cloned()
                .unwrap_or_else(ToolRegistration::official_native);
            let scope = Self::effective_scope(&metadata);
            if scope.allows_parent() {
                parent.push(tool.definition());
            }
            if scope.allows_child() {
                child.push(tool.definition());
            }
        }
        parent.sort_by(|a, b| a.name.cmp(&b.name));
        child.sort_by(|a, b| a.name.cmp(&b.name));
        self.definitions = parent.clone();
        self.parent_definitions = parent;
        self.child_definitions = child;
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Return all tool definitions (for injection into the LLM system prompt).
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub fn definitions_for(&self, context: ToolProfileContext) -> &[ToolDefinition] {
        match context {
            ToolProfileContext::Parent => &self.parent_definitions,
            ToolProfileContext::Child => &self.child_definitions,
        }
    }

    /// List all registered tool names.
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Notify all registered tools that the active session key changed.
    pub fn set_session_key(&self, session_key: &str) {
        for tool in self.tools.values() {
            tool.set_session_key(session_key.to_string());
        }
    }

    /// Execute a tool by name with JSON arguments.
    ///
    /// Runs all registered guards before execution.  The first guard that
    /// returns `Err(reason)` short-circuits — the tool is never invoked and
    /// the reason is returned as `ToolResult { is_error: true }`.
    ///
    /// Empty or whitespace-only argument strings are normalised to `"{}"` to
    /// prevent cryptic `"EOF while parsing a value"` errors from serde_json.
    /// This happens when an LLM returns a tool call with no argument deltas
    /// during SSE streaming.
    pub async fn execute(&self, name: &str, arguments: &str) -> Result<ToolResult, DomainError> {
        let normalised = if arguments.trim().is_empty() {
            "{}"
        } else {
            arguments
        };

        let availability = self
            .metadata
            .get(name)
            .map(|metadata| metadata.availability)
            .unwrap_or(ToolAvailability::Enabled);
        if !availability.is_enabled() {
            return Ok(ToolResult {
                content: format!("tool '{}' is disabled by runtime policy", name),
                is_error: true,
                image_blocks: vec![],
            });
        }
        if let Some(context) = self.execution_profile_context {
            let scope = self
                .metadata
                .get(name)
                .map(Self::effective_scope)
                .unwrap_or(ProfileAvailabilityScope::Both);
            let allowed = match context {
                ToolProfileContext::Parent => scope.allows_parent(),
                ToolProfileContext::Child => scope.allows_child(),
            };
            if !allowed {
                return Ok(ToolResult {
                    content: format!(
                        "tool '{}' is not available in the {:?} runtime profile",
                        name, context
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }
        }

        // Run guards before tool execution
        for guard in &self.guards {
            if let Err(reason) = guard.check(name, normalised) {
                return Ok(ToolResult {
                    content: reason,
                    is_error: true,
                    image_blocks: vec![],
                });
            }
        }

        let tool = self
            .get(name)
            .ok_or_else(|| DomainError::Tool(format!("unknown tool: {}", name)))?;
        tool.execute(normalised).await
    }
}

#[path = "registry_traits.rs"]
mod trait_impls;

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "registry_catalogue_tests.rs"]
mod catalogue_tests;
#[cfg(test)]
#[path = "inherited_tool_policy_tests.rs"]
mod inherited_tool_policy_tests;
#[cfg(test)]
#[path = "registry_policy_tests.rs"]
mod policy_tests;
#[cfg(test)]
#[path = "registry_stable_id_tests.rs"]
mod stable_id_tests;
