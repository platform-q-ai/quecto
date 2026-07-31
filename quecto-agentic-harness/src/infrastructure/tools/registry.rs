// ToolRegistry: holds all Tool implementations, provides lookup by name.

use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{
    ExtensionToolRegistry, SessionAwareTools, Tool, ToolCatalog, ToolDefinition, ToolExecutor,
    ToolGuard, ToolResult,
};
use crate::domain::tool_descriptor::{ToolAvailability, ToolDescriptor, ToolSource};
use crate::infrastructure::config::Config;
use crate::infrastructure::security::sandbox::Sandbox;

use super::bash::ExecOptions;

/// Ownership and lifecycle metadata supplied when a tool enters the common
/// registry. Delivery adapters (bundled native, UDS, future sources) differ only
/// in this metadata and in the concrete `Tool` implementation/proxy they supply.
#[derive(Debug, Clone)]
pub struct ToolRegistration {
    pub source: ToolSource,
    pub owner: Cow<'static, str>,
    pub availability: ToolAvailability,
    /// Whether lifecycle APIs may unregister this concrete registration without
    /// removing/denying the stable tool name. UDS tools are unloadable when their
    /// connection unregisters or disconnects; bundled native tools are not.
    pub unloadable: bool,
}

impl ToolRegistration {
    pub fn official_native() -> Self {
        Self {
            source: ToolSource::BundledNative,
            owner: Cow::Borrowed("quecto:official-tools"),
            availability: ToolAvailability::Enabled,
            unloadable: false,
        }
    }

    pub fn uds() -> Self {
        Self::uds_owner("uds:runtime")
    }

    pub fn uds_owner(owner: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: ToolSource::Uds,
            owner: owner.into(),
            availability: ToolAvailability::Enabled,
            unloadable: true,
        }
    }

    pub fn runtime(owner: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: ToolSource::Runtime,
            owner: owner.into(),
            availability: ToolAvailability::Enabled,
            unloadable: true,
        }
    }

    pub fn with_availability(mut self, availability: ToolAvailability) -> Self {
        self.availability = availability;
        self
    }

    pub fn unloadable(mut self, unloadable: bool) -> Self {
        self.unloadable = unloadable;
        self
    }
}

/// Registry of all available tools, keyed by name.
pub struct ToolRegistryImpl {
    tools: HashMap<String, Arc<dyn Tool>>,
    metadata: HashMap<String, ToolRegistration>,
    definitions: Vec<ToolDefinition>,
    guards: Vec<Arc<dyn ToolGuard>>,
    /// Names explicitly reserved away from future registration.
    ///
    /// Destructive `remove()` / `remove_all()` add names here while unregistering
    /// their descriptors. Startup policy restrictions add names here without
    /// unregistering existing tools, so disabled names remain described but UDS
    /// and other runtime registration paths cannot reintroduce or shadow them.
    denied_names: std::collections::HashSet<String>,
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
            guards: Vec::new(),
            denied_names: std::collections::HashSet::new(),
        }
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

    /// Create a registry with the core filesystem and exec tools (default options).
    pub fn with_core_tools(workspace: PathBuf, sandbox: Sandbox) -> Self {
        Self::with_core_tools_and_exec_options(workspace, sandbox, ExecOptions::default())
    }

    /// Create a registry with core tools and an explicit exec capture limit.
    pub fn with_core_tools_and_exec_settings(
        workspace: PathBuf,
        sandbox: Sandbox,
        max_capture_bytes: usize,
    ) -> Self {
        let exec_options = ExecOptions {
            max_capture_bytes,
            ..ExecOptions::default()
        };
        Self::with_core_tools_and_exec_options(workspace, sandbox, exec_options)
    }

    /// Create a registry with core tools and explicit exec options.
    pub fn with_core_tools_and_exec_options(
        workspace: PathBuf,
        sandbox: Sandbox,
        exec_options: ExecOptions,
    ) -> Self {
        Self::with_core_tools_and_exec_options_spawned(workspace, sandbox, exec_options, false)
    }

    /// Create a registry with core tools, exec options, and docs visibility for
    /// top-level vs spawned agents (#1319).
    pub fn with_core_tools_and_exec_options_spawned(
        workspace: PathBuf,
        sandbox: Sandbox,
        exec_options: ExecOptions,
        spawned: bool,
    ) -> Self {
        crate::infrastructure::extensions::native::build_official_tool_registry(
            workspace,
            sandbox,
            exec_options,
            spawned,
        )
    }

    /// Remove a tool by name and permanently block re-registration.
    ///
    /// Returns `true` if the tool was found and removed, `false` otherwise.
    /// The name is added to the denylist so bundled native and runtime-loadable
    /// registration paths will reject it.
    pub fn remove(&mut self, name: &str) -> bool {
        self.denied_names.insert(name.to_string());
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
            self.denied_names.insert(name.clone());
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

    /// Apply startup `--disable-tool` policy.
    ///
    /// Existing tools are disabled but remain registered/described. Every named
    /// tool, including unknown names, is also denied for future registration so
    /// UDS clients cannot reintroduce a process-disabled capability later.
    /// Returns unknown names for caller-visible warnings.
    pub fn apply_startup_tool_restrictions(&mut self, names: &[String]) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut rebuild_needed = false;
        for name in names {
            self.deny_registration_name(name);
            if !self.tools.contains_key(name) {
                warnings.push(name.clone());
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
    pub fn register_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_with_metadata(tool, ToolRegistration::runtime("runtime:extension"))
    }

    /// Register a UDS-delivered extension tool.
    pub fn register_uds_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_with_metadata(tool, ToolRegistration::uds())
    }

    /// Return whether a UDS-delivered extension tool with `name` and `owner`
    /// would be accepted by the registry without mutating it.
    pub fn can_register_uds_extension_for_owner(&self, name: &str, owner: &str) -> bool {
        if self.denied_names.contains(name) {
            return false;
        }
        if let Some(existing) = self.metadata.get(name) {
            existing.unloadable && existing.owner.as_ref() == owner
        } else {
            true
        }
    }

    /// Register a UDS-delivered extension tool with per-connection ownership.
    pub fn register_uds_extension_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
    ) -> bool {
        self.register_with_metadata(tool, ToolRegistration::uds_owner(owner))
    }

    /// Remove an unloadable tool by name.
    ///
    /// No-op if `name` is not unloadable, preventing runtime lifecycle events
    /// from removing bundled native official tools.
    pub fn unregister_extension(&mut self, name: &str) {
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
    pub fn unregister_extensions_for_owner(&mut self, owner: &str) -> Vec<String> {
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
    pub fn extension_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .metadata
            .iter()
            .filter(|(_, metadata)| metadata.unloadable)
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
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

    /// Runtime-disable a registered tool without removing its descriptor.
    pub fn disable_tool(&mut self, name: &str) -> bool {
        self.set_availability(name, ToolAvailability::Disabled)
    }

    /// Runtime-enable a registered tool without restart.
    pub fn enable_tool(&mut self, name: &str) -> bool {
        self.set_availability(name, ToolAvailability::Enabled)
    }

    fn set_availability(&mut self, name: &str, availability: ToolAvailability) -> bool {
        if !self.tools.contains_key(name) {
            return false;
        }
        let metadata = self
            .metadata
            .entry(name.to_string())
            .or_insert_with(ToolRegistration::official_native);
        if metadata.availability == availability {
            return true;
        }
        metadata.availability = availability;
        self.rebuild_definitions();
        true
    }

    /// Rebuild the cached definitions list from enabled registered tools.
    ///
    /// Deduplication is unnecessary: `self.tools` is a `HashMap<String, _>`
    /// keyed by `tool.definition().name`, so keys are inherently unique.
    fn rebuild_definitions(&mut self) {
        self.definitions = self
            .tools
            .iter()
            .filter(|(name, _)| {
                self.metadata
                    .get(*name)
                    .map(|metadata| metadata.availability.is_enabled())
                    .unwrap_or(true)
            })
            .map(|(_, tool)| tool.definition())
            .collect();
        self.definitions.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Return all tool definitions (for injection into the LLM system prompt).
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
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

impl ToolCatalog for ToolRegistryImpl {
    fn definitions(&self) -> &[ToolDefinition] {
        self.definitions()
    }

    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.descriptors()
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

impl ExtensionToolRegistry for ToolRegistryImpl {
    fn extension_names(&self) -> Vec<String> {
        self.extension_names()
    }

    fn register_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_extension(tool)
    }

    fn unregister_extension(&mut self, name: &str) {
        self.unregister_extension(name);
    }

    fn unregister_extensions_for_owner(&mut self, owner: &str) -> Vec<String> {
        self.unregister_extensions_for_owner(owner)
    }

    fn register_uds_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_uds_extension(tool)
    }

    fn can_register_uds_extension_for_owner(&self, name: &str, owner: &str) -> bool {
        self.can_register_uds_extension_for_owner(name, owner)
    }

    fn register_uds_extension_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
    ) -> bool {
        self.register_uds_extension_for_owner(tool, owner)
    }

    fn enable_tool(&mut self, name: &str) -> bool {
        self.enable_tool(name)
    }

    fn disable_tool(&mut self, name: &str) -> bool {
        self.disable_tool(name)
    }
}

impl SessionAwareTools for ToolRegistryImpl {
    fn set_session_key(&self, session_key: &str) {
        self.set_session_key(session_key);
    }
}

impl crate::domain::tool::ToolRegistry for ToolRegistryImpl {}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
