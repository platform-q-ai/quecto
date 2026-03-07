// ToolRegistry: holds all Tool implementations, provides lookup by name.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolGuard, ToolRegistry, ToolResult};
use crate::infrastructure::config::{Config, ExecIsolationConfig};
use crate::infrastructure::security::sandbox::Sandbox;

use super::bash::{ExecIsolationMode, ExecOptions, ExecTool, NsjailOptions};

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
    /// Size of the writable `/tmp` tmpfs inside the jail in MB.
    /// Threaded through from `ExecToolConfig::tmp_size_mb`.
    pub tmp_size_mb: u64,
}
use super::filesystem::{EditTool, LsTool, ReadTool, WriteTool};
use super::find::FindTool;
use super::grep::GrepTool;

/// Registry of all available tools, keyed by name.
pub struct ToolRegistryImpl {
    tools: HashMap<String, Arc<dyn Tool>>,
    definitions: Vec<ToolDefinition>,
    guards: Vec<Arc<dyn ToolGuard>>,
    /// Names of tools that came from extensions (not core).
    /// Used by `sync_extension_tools` to know which tools to remove on reload.
    extension_tool_names: std::collections::HashSet<String>,
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
            tmp_size_mb: exec.tmp_size_mb,
        }
    }

    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            definitions: Vec::new(),
            guards: Vec::new(),
            extension_tool_names: std::collections::HashSet::new(),
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
                tmp_size_mb: Some(settings.tmp_size_mb),
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
        reg.register(Arc::new(ReadTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(WriteTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(EditTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(LsTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(GrepTool::new(workspace.clone(), sandbox.clone())));
        reg.register(Arc::new(FindTool::new(workspace.clone(), sandbox.clone())));

        reg
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let def = tool.definition();
        let name = def.name.clone().into_owned();
        self.tools.insert(name, tool);

        self.rebuild_definitions();
    }

    /// Register a tool as an extension tool (tracked for reload).
    ///
    /// Extension tools can be removed via `unregister_extension` or
    /// replaced in bulk via `sync_extension_tools`.
    pub fn register_extension(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.definition().name.to_string();
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

    /// Synchronise the registry with an `ExtensionRegistry`.
    ///
    /// 1. Remove all currently tracked extension tools.
    /// 2. Add tools from the new `ExtensionRegistry`, rejecting any
    ///    that shadow core tool names.
    pub fn sync_extension_tools(
        &mut self,
        ext_registry: &crate::infrastructure::extensions::registry::ExtensionRegistry,
    ) {
        // Collect core tool names (everything NOT in extension_tool_names)
        let core_names: std::collections::HashSet<String> = self
            .tools
            .keys()
            .filter(|k| !self.extension_tool_names.contains(k.as_str()))
            .cloned()
            .collect();

        // Remove all old extension tools
        for name in std::mem::take(&mut self.extension_tool_names) {
            self.tools.remove(&name);
        }

        // Add new extension tools, rejecting shadows
        for tool in ext_registry.all_tools() {
            let name = tool.definition().name.to_string();
            if core_names.contains(&name) {
                tracing::warn!(tool = %name, "extension tool rejected: shadows core tool");
                continue;
            }
            self.extension_tool_names.insert(name.clone());
            self.tools.insert(name, tool);
        }

        self.rebuild_definitions();
    }

    /// Return the names of currently registered extension tools.
    pub fn extension_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.extension_tool_names.iter().cloned().collect();
        names.sort();
        names
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
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(names.contains(&"write".to_string()));
        assert!(names.contains(&"edit".to_string()));
        assert!(names.contains(&"ls".to_string()));
    }

    #[test]
    fn test_registry_get_returns_tool() {
        let (reg, _tmp) = test_registry();
        assert!(reg.get("bash").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_definitions() {
        let (reg, _tmp) = test_registry();
        let defs = reg.definitions();
        assert_eq!(defs.len(), 7); // bash, read, write, edit, ls, grep, find
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

    // --- Fix 1: Empty argument normalisation ---

    #[tokio::test]
    async fn test_execute_empty_string_args_normalised() {
        // Empty string "" should be normalised to "{}" so tools don't get
        // "EOF while parsing a value" from serde_json::from_str("").
        let (reg, _tmp) = test_registry();
        // ls accepts empty args (defaults to ".") so this should succeed
        let result = reg.execute("ls", "").await;
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
        let tr = result.unwrap();
        assert!(!tr.is_error, "expected non-error, got: {}", tr.content);
    }

    #[tokio::test]
    async fn test_execute_whitespace_only_args_normalised() {
        let (reg, _tmp) = test_registry();
        let result = reg.execute("ls", "   ").await;
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
        let tr = result.unwrap();
        assert!(!tr.is_error, "expected non-error, got: {}", tr.content);
    }

    #[tokio::test]
    async fn test_execute_empty_args_no_eof_error() {
        // Tools with required params should return actionable error, not EOF parse error
        let (reg, _tmp) = test_registry();
        let result = reg.execute("read", "").await.unwrap();
        assert!(result.is_error, "expected error result");
        assert!(
            !result.content.contains("EOF while parsing"),
            "should not contain EOF parse error, got: {}",
            result.content
        );
        assert!(
            result.content.contains("path"),
            "should mention required param, got: {}",
            result.content
        );
    }

    // --- Fix 3: Tool descriptions include usage examples ---

    #[test]
    fn test_all_core_tool_descriptions_include_example() {
        let (reg, _tmp) = test_registry();
        let defs = reg.definitions();
        for def in defs {
            assert!(
                def.description.contains("Example"),
                "tool '{}' description should contain an Example, got: {}",
                def.name,
                def.description
            );
        }
    }

    // --- #210: definitions() returns borrowed slice ---

    #[test]
    fn test_definitions_returns_borrowed_slice() {
        let (reg, _tmp) = test_registry();
        // definitions() should return &[ToolDefinition], not Vec<ToolDefinition>.
        // This test verifies it compiles as a slice reference.
        let defs: &[ToolDefinition] = reg.definitions();
        assert_eq!(defs.len(), 7);
    }

    #[test]
    fn test_trait_definitions_returns_borrowed_slice() {
        let (reg, _tmp) = test_registry();
        let trait_reg: &dyn ToolRegistry = &reg;
        let defs: &[ToolDefinition] = trait_reg.definitions();
        assert!(!defs.is_empty());
    }

    // --- #214: tool_count() method ---

    #[test]
    fn test_tool_count_returns_correct_count() {
        let (reg, _tmp) = test_registry();
        let trait_reg: &dyn ToolRegistry = &reg;
        assert_eq!(trait_reg.tool_count(), 7);
    }

    #[test]
    fn test_tool_count_empty_registry() {
        let reg = ToolRegistryImpl::new();
        let trait_reg: &dyn ToolRegistry = &reg;
        assert_eq!(trait_reg.tool_count(), 0);
    }

    // --- #215: rebuild_definitions works without HashSet ---

    // --- #318: Extension tool tracking ---

    #[test]
    fn test_register_extension_tool() {
        let mut reg = ToolRegistryImpl::new();
        let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("ext_greet"));
        reg.register_extension(tool);
        assert!(reg.get("ext_greet").is_some());
        assert!(reg.extension_names().contains(&"ext_greet".to_string()));
    }

    #[test]
    fn test_register_extension_tool_does_not_mark_as_core() {
        let (mut reg, _tmp) = test_registry();
        let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("ext_greet"));
        reg.register_extension(tool);
        // Core tools should not appear in extension_names
        assert!(!reg.extension_names().contains(&"bash".to_string()));
        assert!(reg.extension_names().contains(&"ext_greet".to_string()));
    }

    #[test]
    fn test_unregister_extension_tool() {
        let mut reg = ToolRegistryImpl::new();
        let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("ext_greet"));
        reg.register_extension(tool);
        assert!(reg.get("ext_greet").is_some());

        reg.unregister_extension("ext_greet");
        assert!(reg.get("ext_greet").is_none());
        assert!(!reg.extension_names().contains(&"ext_greet".to_string()));
    }

    #[test]
    fn test_unregister_extension_does_not_remove_core_tools() {
        let (mut reg, _tmp) = test_registry();
        // Attempting to unregister a core tool via unregister_extension should be a no-op
        reg.unregister_extension("bash");
        assert!(reg.get("bash").is_some(), "core tool should not be removed");
    }

    #[test]
    fn test_sync_extension_tools_adds_new() {
        let (mut reg, _tmp) = test_registry();
        let initial_count = reg.definitions().len();

        let mut ext_reg = crate::infrastructure::extensions::registry::ExtensionRegistry::new();
        ext_reg.register(Arc::new(TestExtensionForRegistry {
            name: "ext1".into(),
            tool: Arc::new(DummyTestTool::new("ext1")),
        }));

        reg.sync_extension_tools(&ext_reg);
        assert_eq!(reg.definitions().len(), initial_count + 1);
        assert!(reg.get("ext1").is_some());
        assert!(reg.extension_names().contains(&"ext1".to_string()));
    }

    #[test]
    fn test_sync_extension_tools_removes_stale() {
        let (mut reg, _tmp) = test_registry();
        let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("old_ext"));
        reg.register_extension(tool);
        assert!(reg.get("old_ext").is_some());

        // Sync with empty registry — old_ext should be removed
        let ext_reg = crate::infrastructure::extensions::registry::ExtensionRegistry::new();
        reg.sync_extension_tools(&ext_reg);
        assert!(reg.get("old_ext").is_none());
    }

    #[test]
    fn test_sync_extension_tools_rejects_shadow() {
        let (mut reg, _tmp) = test_registry();
        let initial_count = reg.definitions().len();

        let mut ext_reg = crate::infrastructure::extensions::registry::ExtensionRegistry::new();
        ext_reg.register(Arc::new(TestExtensionForRegistry {
            name: "bash".into(), // shadows core tool
            tool: Arc::new(DummyTestTool::new("bash")),
        }));

        reg.sync_extension_tools(&ext_reg);
        // Should NOT have added the shadowing extension
        assert_eq!(reg.definitions().len(), initial_count);
        assert!(!reg.extension_names().contains(&"bash".to_string()));
    }

    #[test]
    fn test_sync_extension_tools_replaces_existing() {
        let (mut reg, _tmp) = test_registry();

        // First sync: add ext1
        let mut ext_reg1 = crate::infrastructure::extensions::registry::ExtensionRegistry::new();
        ext_reg1.register(Arc::new(TestExtensionForRegistry {
            name: "ext1".into(),
            tool: Arc::new(DummyTestTool::new("ext1")),
        }));
        reg.sync_extension_tools(&ext_reg1);
        assert!(reg.get("ext1").is_some());

        // Second sync: replace with ext2 only
        let mut ext_reg2 = crate::infrastructure::extensions::registry::ExtensionRegistry::new();
        ext_reg2.register(Arc::new(TestExtensionForRegistry {
            name: "ext2".into(),
            tool: Arc::new(DummyTestTool::new("ext2")),
        }));
        reg.sync_extension_tools(&ext_reg2);
        assert!(reg.get("ext1").is_none(), "ext1 should be removed");
        assert!(reg.get("ext2").is_some(), "ext2 should be added");
    }

    #[test]
    fn test_extension_names_empty_by_default() {
        let (reg, _tmp) = test_registry();
        assert!(reg.extension_names().is_empty());
    }

    /// Minimal test tool for extension tracking tests.
    struct DummyTestTool {
        name: String,
    }

    impl DummyTestTool {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
            }
        }
    }

    impl Tool for DummyTestTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.clone().into(),
                description: format!("Test tool {}", self.name).into(),
                parameters_schema: r#"{"type":"object"}"#.into(),
            }
        }

        fn execute(
            &self,
            _arguments: &str,
        ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
            Box::pin(async {
                Ok(ToolResult {
                    content: "ok".into(),
                    is_error: false,
                    image_blocks: vec![],
                })
            })
        }
    }

    /// Minimal extension for sync_extension_tools tests.
    struct TestExtensionForRegistry {
        name: String,
        tool: Arc<dyn Tool>,
    }

    impl crate::domain::extension::Extension for TestExtensionForRegistry {
        fn name(&self) -> &str {
            &self.name
        }
        fn tools(&self) -> Vec<Arc<dyn Tool>> {
            vec![self.tool.clone()]
        }
    }

    #[test]
    fn test_rebuild_definitions_no_duplicates_after_re_register() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), true);
        let mut reg = ToolRegistryImpl::with_core_tools(tmp.path().to_path_buf(), sandbox.clone());
        let initial_count = reg.definitions().len();

        // Re-register a tool that already exists — should not duplicate
        reg.register(Arc::new(
            crate::infrastructure::tools::filesystem::ReadTool::new(
                Arc::new(tmp.path().to_path_buf()),
                Arc::new(sandbox),
            ),
        ));
        assert_eq!(
            reg.definitions().len(),
            initial_count,
            "re-registering a tool should not create duplicates"
        );
    }
}
