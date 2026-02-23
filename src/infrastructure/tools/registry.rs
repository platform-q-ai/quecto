// ToolRegistry: holds all Tool implementations, provides lookup by name.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use crate::infrastructure::config::{Config, ExecIsolationConfig};
use crate::infrastructure::security::sandbox::Sandbox;

use super::exec::{ExecIsolationMode, ExecOptions, ExecTool, NsjailOptions};

#[derive(Debug, Clone)]
pub struct ExecRegistrySettings {
    pub max_capture_bytes: usize,
    pub isolation_mode: ExecIsolationMode,
    pub allow_native_fallback: bool,
    pub nsjail_binary: String,
    pub network_passthrough: bool,
    pub memory_limit_mb: u64,
    pub pid_limit: u64,
    pub cpu_time_limit_secs: u64,
    pub wall_time_limit_secs: u64,
    pub die_with_parent: bool,
}
use super::filesystem::{AppendFileTool, EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};

/// Registry of all available tools, keyed by name.
pub struct ToolRegistryImpl {
    tools: HashMap<String, Arc<dyn Tool>>,
    definitions: Vec<ToolDefinition>,
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
    /// Build exec registry settings from config in one place.
    pub fn exec_registry_settings_from_config(config: &Config) -> ExecRegistrySettings {
        let exec = &config.tools.exec;
        ExecRegistrySettings {
            max_capture_bytes: config.agents.defaults.exec_max_capture_bytes,
            isolation_mode: if exec.isolation == ExecIsolationConfig::Nsjail {
                ExecIsolationMode::Nsjail
            } else {
                ExecIsolationMode::Native
            },
            allow_native_fallback: exec.allow_native_fallback,
            nsjail_binary: exec.nsjail_binary.clone(),
            network_passthrough: exec.network_passthrough,
            memory_limit_mb: exec.memory_limit_mb,
            pid_limit: exec.pid_limit,
            cpu_time_limit_secs: exec.cpu_time_limit_secs,
            wall_time_limit_secs: exec.wall_time_limit_secs,
            die_with_parent: exec.die_with_parent,
        }
    }

    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            definitions: Vec::new(),
        }
    }

    /// Create a registry with the core filesystem and exec tools.
    pub fn with_core_tools(workspace: PathBuf, sandbox: Sandbox) -> Self {
        Self::with_core_tools_and_exec_capture_bytes(workspace, sandbox, 1024 * 1024)
    }

    /// Create a registry with core tools and configurable exec output capture bytes.
    pub fn with_core_tools_and_exec_capture_bytes(
        workspace: PathBuf,
        sandbox: Sandbox,
        exec_max_capture_bytes: usize,
    ) -> Self {
        let exec_options = ExecOptions {
            max_capture_bytes: exec_max_capture_bytes,
            ..ExecOptions::default()
        };
        Self::with_core_tools_and_exec_options(workspace, sandbox, exec_options)
    }

    /// Create a registry with core tools and exec isolation mode settings.
    pub fn with_core_tools_and_exec_settings(
        workspace: PathBuf,
        sandbox: Sandbox,
        settings: ExecRegistrySettings,
    ) -> Self {
        let exec_options = ExecOptions {
            max_capture_bytes: settings.max_capture_bytes,
            isolation_mode: settings.isolation_mode,
            allow_native_fallback: settings.allow_native_fallback,
            nsjail: NsjailOptions {
                binary: settings.nsjail_binary,
                network_passthrough: settings.network_passthrough,
                memory_limit_mb: Some(settings.memory_limit_mb),
                pid_limit: Some(settings.pid_limit),
                cpu_time_limit_secs: Some(settings.cpu_time_limit_secs),
                wall_time_limit_secs: Some(settings.wall_time_limit_secs),
                die_with_parent: settings.die_with_parent,
            },
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
        reg.register(Arc::new(ReadFileTool::new(
            workspace.clone(),
            sandbox.clone(),
        )));
        reg.register(Arc::new(WriteFileTool::new(
            workspace.clone(),
            sandbox.clone(),
        )));
        reg.register(Arc::new(EditFileTool::new(
            workspace.clone(),
            sandbox.clone(),
        )));
        reg.register(Arc::new(AppendFileTool::new(
            workspace.clone(),
            sandbox.clone(),
        )));
        reg.register(Arc::new(ListDirTool::new(
            workspace.clone(),
            sandbox.clone(),
        )));

        reg
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let def = tool.definition();
        let name = def.name.clone();
        self.tools.insert(name, tool);

        self.definitions = self.tools.values().map(|t| t.definition()).collect();
        self.definitions.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Return all tool definitions (for injection into the LLM system prompt).
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.definitions.clone()
    }

    /// List all registered tool names.
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Execute a tool by name with JSON arguments.
    pub async fn execute(&self, name: &str, arguments: &str) -> Result<ToolResult, DomainError> {
        let tool = self
            .get(name)
            .ok_or_else(|| DomainError::Tool(format!("unknown tool: {}", name)))?;
        tool.execute(arguments).await
    }
}

impl ToolRegistry for ToolRegistryImpl {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.definitions()
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
mod tests {
    use super::*;
    use crate::infrastructure::security::sandbox::Sandbox;
    use tempfile::TempDir;

    fn test_registry() -> (ToolRegistryImpl, TempDir) {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), true);
        let reg = ToolRegistryImpl::with_core_tools(tmp.path().to_path_buf(), sandbox);
        (reg, tmp)
    }

    #[test]
    fn test_registry_contains_core_tools() {
        let (reg, _tmp) = test_registry();
        let names = reg.names();
        assert!(names.contains(&"exec".to_string()));
        assert!(names.contains(&"read_file".to_string()));
        assert!(names.contains(&"write_file".to_string()));
        assert!(names.contains(&"edit_file".to_string()));
        assert!(names.contains(&"append_file".to_string()));
        assert!(names.contains(&"list_dir".to_string()));
    }

    #[test]
    fn test_registry_get_returns_tool() {
        let (reg, _tmp) = test_registry();
        assert!(reg.get("exec").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_definitions() {
        let (reg, _tmp) = test_registry();
        let defs = reg.definitions();
        assert_eq!(defs.len(), 6);
    }

    #[tokio::test]
    async fn test_execute_unknown_tool() {
        let (reg, _tmp) = test_registry();
        let result = reg.execute("nonexistent", "{}").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown tool"));
    }

    #[test]
    fn test_empty_registry() {
        let reg = ToolRegistryImpl::new();
        assert!(reg.names().is_empty());
        assert!(reg.definitions().is_empty());
        assert!(reg.get("anything").is_none());
    }

    #[test]
    fn test_debug_format() {
        let (reg, _tmp) = test_registry();
        let debug = format!("{:?}", reg);
        assert!(debug.contains("ToolRegistryImpl"));
    }

    #[test]
    fn test_default_creates_empty() {
        let reg = ToolRegistryImpl::default();
        assert!(reg.names().is_empty());
    }

    #[tokio::test]
    async fn test_trait_execute() {
        let (reg, _tmp) = test_registry();
        // Test through the ToolRegistry trait (dyn dispatch)
        let trait_reg: &dyn ToolRegistry = &reg;
        let defs = trait_reg.definitions();
        assert!(!defs.is_empty());

        // Execute through trait
        let result = trait_reg.execute("nonexistent", "{}").await;
        assert!(result.is_err());
    }
}
