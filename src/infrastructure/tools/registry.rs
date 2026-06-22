// ToolRegistry: holds all Tool implementations, provides lookup by name.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolGuard, ToolRegistry, ToolResult};
use crate::infrastructure::config::Config;
use crate::infrastructure::security::sandbox::Sandbox;

use super::bash::{ExecOptions, ExecTool};

use super::docs::DocsTool;
use super::filesystem::{EditTool, LsTool, ReadTool, WriteTool};
use super::find::FindTool;
use super::grep::GrepTool;

/// Registry of all available tools, keyed by name.
pub struct ToolRegistryImpl {
    tools: HashMap<String, Arc<dyn Tool>>,
    definitions: Vec<ToolDefinition>,
    guards: Vec<Arc<dyn ToolGuard>>,
    /// Names of tools that came from extensions (not core).
    /// Tracks which tools `unregister_extension` may remove (vs core tools).
    extension_tool_names: std::collections::HashSet<String>,
    /// Names explicitly removed via `remove()` or `remove_all()`.
    /// These are permanently blocked from re-registration via
    /// `register()` and `register_extension()`.
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
            definitions: Vec::new(),
            guards: Vec::new(),
            extension_tool_names: std::collections::HashSet::new(),
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
        let sandbox = Arc::new(sandbox);
        let workspace = Arc::new(workspace);
        let mut reg = Self::new();

        reg.register(Arc::new(ExecTool::with_options(
            workspace.clone(),
            sandbox.clone(),
            exec_options,
        )));
        reg.register(Arc::new(ReadTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(WriteTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(EditTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(LsTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(GrepTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(FindTool::new(workspace.clone(), sandbox.clone())));
        // Quecto's own capability docs, embedded in the binary — reachable from
        // any working directory (not the filesystem).
        reg.register(Arc::new(DocsTool::new()));

        reg
    }

    /// Remove a tool by name and permanently block re-registration.
    ///
    /// Returns `true` if the tool was found and removed, `false` otherwise.
    /// The name is added to the denylist so `register()` and
    /// `register_extension()` will reject it.
    pub fn remove(&mut self, name: &str) -> bool {
        self.denied_names.insert(name.to_string());
        if self.tools.remove(name).is_some() {
            self.extension_tool_names.remove(name);
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
            self.extension_tool_names.remove(name.as_str());
        }
        if warnings.len() < names.len() {
            // At least one tool was actually removed
            self.rebuild_definitions();
        }
        warnings
    }

    /// Register a tool. No-op if the name is on the denylist.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let def = tool.definition();
        let name = def.name.clone().into_owned();
        if self.denied_names.contains(&name) {
            tracing::debug!(tool = %name, "register rejected: tool is on the denylist");
            return;
        }
        self.tools.insert(name, tool);

        self.rebuild_definitions();
    }

    /// Register a tool as an extension tool (tracked for reload).
    ///
    /// Extension tools can be removed via `unregister_extension`.
    /// Rejects tools that shadow core tool names.
    pub fn register_extension(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.definition().name.to_string();
        if self.denied_names.contains(&name) {
            tracing::warn!(tool = %name, "register_extension rejected: tool is on the denylist");
            return;
        }
        // Reject if name exists and is NOT already an extension tool (i.e. it's core)
        if self.tools.contains_key(&name) && !self.extension_tool_names.contains(&name) {
            tracing::warn!(tool = %name, "register_extension rejected: shadows core tool");
            return;
        }
        self.extension_tool_names.insert(name.clone());
        self.tools.insert(name, tool);
        self.rebuild_definitions();
    }

    /// Remove an extension tool by name.
    ///
    /// No-op if `name` is not in the extension set (prevents removing core tools).
    pub fn unregister_extension(&mut self, name: &str) {
        if !self.extension_tool_names.contains(name) {
            return;
        }
        self.extension_tool_names.remove(name);
        self.tools.remove(name);
        self.rebuild_definitions();
    }

    /// Return the names of currently registered extension tools.
    pub fn extension_names(&self) -> Vec<String> {
        self.extension_tool_names.iter().cloned().collect()
    }

    /// Rebuild the cached definitions list from all registered tools.
    ///
    /// Deduplication is unnecessary: `self.tools` is a `HashMap<String, _>`
    /// keyed by `tool.definition().name`, so keys are inherently unique.
    fn rebuild_definitions(&mut self) {
        self.definitions = self.tools.values().map(|t| t.definition()).collect();
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

impl ToolRegistry for ToolRegistryImpl {
    fn definitions(&self) -> &[ToolDefinition] {
        self.definitions()
    }

    fn extension_names(&self) -> Vec<String> {
        self.extension_names()
    }

    fn set_session_key(&self, session_key: &str) {
        self.set_session_key(session_key);
    }

    fn register_extension(&mut self, tool: Arc<dyn Tool>) {
        self.register_extension(tool);
    }

    fn unregister_extension(&mut self, name: &str) {
        self.unregister_extension(name);
    }

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

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
