// ToolRegistry: holds all Tool implementations, provides lookup by name.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::exec::ExecTool;
use super::filesystem::{AppendFileTool, EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};

/// Registry of all available tools, keyed by name.
pub struct ToolRegistryImpl {
    tools: HashMap<String, Arc<dyn Tool>>,
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
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Create a registry with the core filesystem and exec tools.
    pub fn with_core_tools(workspace: PathBuf, sandbox: Sandbox) -> Self {
        let sandbox = Arc::new(sandbox);
        let workspace = Arc::new(workspace);
        let mut reg = Self::new();

        reg.register(Arc::new(ExecTool::new(workspace.clone(), sandbox.clone())));
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
        let name = tool.definition().name;
        self.tools.insert(name, tool);
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Return all tool definitions (for injection into the LLM system prompt).
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
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
