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
    assert!(names.contains(&"rust_ast_graph".to_string()));
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
    assert_eq!(defs.len(), 9); // bash, read, write, edit, ls, grep, find, rust_ast_graph, docs
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
    assert_eq!(defs.len(), 9);
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
    assert_eq!(trait_reg.tool_count(), 9);
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
fn test_register_extension_rejects_shadow_of_core_tool() {
    let (mut reg, _tmp) = test_registry();
    let initial_count = reg.definitions().len();
    let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("bash")); // shadows core
    reg.register_extension(tool);
    // Should NOT have replaced the core tool or added to extension_names
    assert_eq!(reg.definitions().len(), initial_count);
    assert!(!reg.extension_names().contains(&"bash".to_string()));
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

// --- #402: remove() method ---

#[test]
fn test_remove_core_tool() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.get("bash").is_some());
    let before = reg.definitions().len();

    let removed = reg.remove("bash");
    assert!(removed, "remove should return true for existing tool");
    assert!(reg.get("bash").is_none(), "bash should be gone");
    assert_eq!(reg.definitions().len(), before - 1);
}

#[test]
fn test_remove_nonexistent_tool() {
    let (mut reg, _tmp) = test_registry();
    let before = reg.definitions().len();

    let removed = reg.remove("nonexistent");
    assert!(!removed, "remove should return false for unknown tool");
    assert_eq!(reg.definitions().len(), before);
}

#[test]
fn test_remove_extension_tool() {
    let mut reg = ToolRegistryImpl::new();
    let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("ext_greet"));
    reg.register_extension(tool);
    assert!(reg.get("ext_greet").is_some());

    let removed = reg.remove("ext_greet");
    assert!(removed);
    assert!(reg.get("ext_greet").is_none());
    // Should also clean up extension_tool_names tracking
    assert!(!reg.extension_names().contains(&"ext_greet".to_string()));
}

#[test]
fn test_remove_blocks_re_registration() {
    let (mut reg, _tmp) = test_registry();
    reg.remove("bash");
    assert!(reg.get("bash").is_none());

    // Attempt to re-register via register()
    let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("bash"));
    reg.register(tool);
    assert!(
        reg.get("bash").is_none(),
        "denylist should block register()"
    );

    // Attempt via register_extension()
    let tool2: Arc<dyn Tool> = Arc::new(DummyTestTool::new("bash"));
    reg.register_extension(tool2);
    assert!(
        reg.get("bash").is_none(),
        "denylist should block register_extension()"
    );
}

#[test]
fn test_remove_all_batch() {
    let (mut reg, _tmp) = test_registry();
    let before = reg.definitions().len();

    let warnings = reg.remove_all(&["bash".into(), "read".into(), "nonexistent".into()]);
    assert_eq!(warnings, vec!["nonexistent"]);
    assert_eq!(reg.definitions().len(), before - 2);
    assert!(reg.get("bash").is_none());
    assert!(reg.get("read").is_none());
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

#[tokio::test]
async fn dummy_test_tool_trait_surface_defaults_are_exercised() {
    let tool = DummyTestTool::new("surface_tool");

    tool.set_session_key("session-key".into());
    assert_eq!(tool.definition().name, "surface_tool");
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert_eq!(result.content, "ok");
    assert!(!result.is_error);
}
