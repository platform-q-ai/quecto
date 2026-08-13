use super::*;
use crate::domain::tool::ToolRegistry;
use crate::domain::tool_descriptor::ToolSource;
use crate::infrastructure::security::sandbox::Sandbox;
use std::pin::Pin;
use tempfile::TempDir;

pub(crate) fn test_registry() -> (ToolRegistryImpl, TempDir) {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), true);
    let reg = crate::infrastructure::extensions::native::build_official_tool_registry(
        tmp.path().to_path_buf(),
        sandbox,
        Default::default(),
    );
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
    assert_eq!(defs.len(), 9); // bash, read, write, edit, ls, grep, find, docs
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

// --- #1276 Phase 2: unified metadata registration with unloadable lifecycle ---

#[test]
fn register_with_metadata_controls_descriptor_and_unloadability() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("phase2_tool")),
        ToolRegistration::runtime("test:owner").with_availability(ToolAvailability::Disabled)
    ));

    let descriptor = reg.descriptor("phase2_tool").expect("descriptor");
    assert_eq!(descriptor.owner.as_ref(), "test:owner");
    assert!(matches!(descriptor.source, ToolSource::Runtime));
    assert!(!descriptor.availability.is_enabled());
    assert!(!reg.definitions().iter().any(|d| d.name == "phase2_tool"));
    assert_eq!(reg.runtime_tool_names(), vec!["phase2_tool".to_string()]);

    assert!(reg.enable_tool("phase2_tool"));
    assert!(reg.definitions().iter().any(|d| d.name == "phase2_tool"));
    reg.unregister_runtime_tool("phase2_tool");
    assert!(reg.get("phase2_tool").is_none());
}

#[test]
fn register_with_metadata_prevents_shadowing_non_unloadable_tools() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("stable")),
        ToolRegistration::official_native()
    ));
    assert!(!reg.register_with_metadata(
        Arc::new(DummyTestTool::new("stable")),
        ToolRegistration::uds()
    ));
    let descriptor = reg.descriptor("stable").expect("descriptor");
    assert!(matches!(descriptor.source, ToolSource::BundledNative));
    assert!(reg.runtime_tool_names().is_empty());
}

#[test]
fn register_with_metadata_replaces_unloadable_tool_for_same_owner() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("dynamic")),
        ToolRegistration::runtime("owner:one")
    ));
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::with_desc("dynamic", "replacement")),
        ToolRegistration::runtime("owner:one")
    ));
    let descriptor = reg.descriptor("dynamic").expect("descriptor");
    assert!(matches!(descriptor.source, ToolSource::Runtime));
    assert_eq!(descriptor.owner.as_ref(), "owner:one");
    assert_eq!(descriptor.definition.description.as_ref(), "replacement");
    assert_eq!(reg.runtime_tool_names(), vec!["dynamic".to_string()]);
}

#[test]
fn legacy_extension_lifecycle_aliases_delegate_to_runtime_and_uds_paths() {
    let mut reg = ToolRegistryImpl::new();
    let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("legacy_ext"));
    assert!(reg.register_extension(tool));
    assert_eq!(reg.extension_names(), vec!["legacy_ext".to_string()]);
    reg.unregister_extension("legacy_ext");
    assert!(reg.extension_names().is_empty());

    let uds_tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("legacy_uds"));
    assert!(reg.can_register_uds_extension_for_owner("legacy_uds", "uds:client:1"));
    assert!(reg.register_uds_extension(uds_tool));
    let owned_tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("legacy_owned"));
    assert!(
        reg.register_uds_extension_for_owner(
            owned_tool,
            std::borrow::Cow::Borrowed("uds:client:2"),
        )
    );
    assert_eq!(
        reg.unregister_extensions_for_owner("uds:client:2"),
        vec!["legacy_owned".to_string()]
    );
}

#[test]
fn test_register_runtime_tool() {
    let mut reg = ToolRegistryImpl::new();
    let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("ext_greet"));
    reg.register_runtime_tool(tool);
    assert!(reg.get("ext_greet").is_some());
    assert!(reg.runtime_tool_names().contains(&"ext_greet".to_string()));
}

#[test]
fn test_register_runtime_tool_does_not_mark_as_core() {
    let (mut reg, _tmp) = test_registry();
    let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("ext_greet"));
    reg.register_runtime_tool(tool);
    // Core tools should not appear in extension_names
    assert!(!reg.runtime_tool_names().contains(&"bash".to_string()));
    assert!(reg.runtime_tool_names().contains(&"ext_greet".to_string()));
}

#[test]
fn test_register_extension_rejects_shadow_of_core_tool() {
    let (mut reg, _tmp) = test_registry();
    let initial_count = reg.definitions().len();
    let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("bash")); // shadows core
    reg.register_runtime_tool(tool);
    // Should NOT have replaced the core tool or added to extension_names
    assert_eq!(reg.definitions().len(), initial_count);
    assert!(!reg.runtime_tool_names().contains(&"bash".to_string()));
}

#[test]
fn test_unregister_runtime_tool() {
    let mut reg = ToolRegistryImpl::new();
    let tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("ext_greet"));
    reg.register_runtime_tool(tool);
    assert!(reg.get("ext_greet").is_some());

    reg.unregister_runtime_tool("ext_greet");
    assert!(reg.get("ext_greet").is_none());
    assert!(!reg.runtime_tool_names().contains(&"ext_greet".to_string()));
}

#[test]
fn test_unregister_extension_does_not_remove_core_tools() {
    let (mut reg, _tmp) = test_registry();
    // Attempting to unregister a core tool via unregister_extension should be a no-op
    reg.unregister_runtime_tool("bash");
    assert!(reg.get("bash").is_some(), "core tool should not be removed");
}

#[test]
fn test_extension_names_empty_by_default() {
    let (reg, _tmp) = test_registry();
    assert!(reg.runtime_tool_names().is_empty());
}

/// Minimal test tool for extension tracking tests.
pub(crate) struct DummyTestTool {
    name: String,
    description: String,
}

impl DummyTestTool {
    pub(crate) fn new(name: &str) -> Self {
        Self::with_desc(name, &format!("Test tool {name}"))
    }

    fn with_desc(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
        }
    }
}

impl Tool for DummyTestTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone().into(),
            description: self.description.clone().into(),
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
    reg.register_runtime_tool(tool);
    assert!(reg.get("ext_greet").is_some());

    let removed = reg.remove("ext_greet");
    assert!(removed);
    assert!(reg.get("ext_greet").is_none());
    // Should also clean up extension_tool_names tracking
    assert!(!reg.runtime_tool_names().contains(&"ext_greet".to_string()));
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
    reg.register_runtime_tool(tool2);
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
    let mut reg = crate::infrastructure::extensions::native::build_official_tool_registry(
        tmp.path().to_path_buf(),
        sandbox.clone(),
        Default::default(),
    );
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

#[test]
fn descriptors_include_source_and_availability_for_native_and_uds_tools() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.descriptors().iter().any(|d| {
        d.definition.name.as_ref() == "bash"
            && matches!(
                d.source,
                crate::domain::tool_descriptor::ToolSource::BundledNative
            )
            && d.availability.is_enabled()
    }));

    let ok = reg.register_uds_tool_for_owner(
        Arc::new(DummyTestTool::new("weather")),
        "uds:client:7".into(),
    );
    assert!(ok);
    let weather = reg.descriptor("weather").expect("weather descriptor");
    assert!(matches!(
        weather.source,
        crate::domain::tool_descriptor::ToolSource::Uds
    ));
    assert_eq!(weather.owner.as_ref(), "uds:client:7");
    assert!(weather.availability.is_enabled());
}

#[test]
fn disable_and_enable_tool_toggle_model_visible_definitions_without_unregister() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.get("bash").is_some());
    assert!(reg.definitions().iter().any(|d| d.name.as_ref() == "bash"));

    assert!(reg.disable_tool("bash"));
    assert!(
        reg.get("bash").is_some(),
        "disabled tool remains registered"
    );
    assert!(
        !reg.definitions().iter().any(|d| d.name.as_ref() == "bash"),
        "disabled tool is hidden from model-visible definitions"
    );
    let disabled = reg.descriptor("bash").expect("bash still has descriptor");
    assert!(!disabled.availability.is_enabled());

    assert!(reg.enable_tool("bash"));
    assert!(reg.definitions().iter().any(|d| d.name.as_ref() == "bash"));
    assert!(reg.descriptor("bash").unwrap().availability.is_enabled());
}

#[tokio::test]
async fn disabled_tool_execute_is_rejected_and_enable_restores_execution() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.disable_tool("ls"));
    let disabled = reg.execute("ls", "{}").await;
    assert!(
        disabled.is_err() || disabled.as_ref().map(|r| r.is_error).unwrap_or(false),
        "disabled tool must not execute successfully"
    );

    assert!(reg.enable_tool("ls"));
    let enabled = reg
        .execute("ls", "{}")
        .await
        .expect("ls executes when enabled");
    assert!(
        !enabled.is_error,
        "enabled tool should execute: {}",
        enabled.content
    );
}

#[test]
fn register_uds_extension_rejects_core_shadow_and_denylist() {
    let (mut reg, _tmp) = test_registry();
    assert!(!reg.register_uds_tool(Arc::new(DummyTestTool::new("bash"))));
    assert!(reg.remove("bash"));
    assert!(!reg.register_uds_tool(Arc::new(DummyTestTool::new("bash"))));
    assert!(reg.get("bash").is_none());
}

#[test]
fn uds_owner_registration_preflight_matches_mutating_acceptance_rules() {
    let (mut reg, _tmp) = test_registry();
    assert!(!reg.can_register_uds_tool_for_owner("bash", "uds:client:1"));
    assert!(reg.remove("bash"));
    assert!(!reg.can_register_uds_tool_for_owner("bash", "uds:client:1"));
    assert!(reg.can_register_uds_tool_for_owner("weather", "uds:client:1"));
    assert!(reg.register_uds_tool_for_owner(
        Arc::new(DummyTestTool::new("weather")),
        "uds:client:1".into(),
    ));
    assert!(reg.can_register_uds_tool_for_owner("weather", "uds:client:1"));
    assert!(!reg.can_register_uds_tool_for_owner("weather", "uds:client:2"));
}

#[test]
fn unloadable_tool_names_remain_owned_by_their_registering_connection() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_uds_tool_for_owner(
        Arc::new(DummyTestTool::new("weather")),
        "uds:client:1".into(),
    ));

    assert!(
        !reg.register_uds_tool_for_owner(
            Arc::new(DummyTestTool::new("weather")),
            "uds:client:2".into(),
        ),
        "a second UDS owner must not replace another client's proxy"
    );
    assert_eq!(
        reg.descriptor("weather").unwrap().owner.as_ref(),
        "uds:client:1"
    );

    assert!(reg.register_uds_tool_for_owner(
        Arc::new(DummyTestTool::with_desc("weather", "new generation")),
        "uds:client:1".into(),
    ));
    let descriptor = reg.descriptor("weather").unwrap();
    assert_eq!(descriptor.owner.as_ref(), "uds:client:1");
    assert_eq!(descriptor.definition.description.as_ref(), "new generation");
}

#[test]
fn owner_scoped_unregister_removes_only_matching_runtime_capabilities() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.register_uds_tool_for_owner(
        Arc::new(DummyTestTool::new("weather")),
        "uds:client:1".into(),
    ));
    assert!(reg.register_uds_tool_for_owner(
        Arc::new(DummyTestTool::new("calendar")),
        "uds:client:2".into(),
    ));

    let removed = reg.unregister_extensions_for_owner("uds:client:1");
    assert_eq!(removed, vec!["weather".to_string()]);
    assert!(reg.get("weather").is_none());
    assert!(reg.get("calendar").is_some());
    assert!(
        reg.get("bash").is_some(),
        "native tools have separate lifecycle"
    );
}

#[test]
fn disable_or_enable_unknown_tool_returns_false() {
    let (mut reg, _tmp) = test_registry();
    assert!(!reg.disable_tool("no_such_tool"));
    assert!(!reg.enable_tool("no_such_tool"));
}

#[test]
fn trait_paths_expose_descriptors_and_runtime_policy() {
    use crate::domain::tool::{RuntimeToolLifecycleRegistry, ToolCatalog};

    let (mut reg, _tmp) = test_registry();
    {
        let catalog: &dyn ToolCatalog = &reg;
        assert!(
            catalog
                .descriptors()
                .iter()
                .any(|d| d.definition.name.as_ref() == "bash")
        );
    }

    {
        let ext: &mut dyn RuntimeToolLifecycleRegistry = &mut reg;
        assert!(ext.disable_tool("bash"));
        assert!(ext.enable_tool("bash"));
        // no-op when already enabled
        assert!(ext.enable_tool("bash"));
        assert!(ext.register_uds_tool(std::sync::Arc::new(DummyTestTool::new("wx"))));
        assert!(ext.runtime_tool_names().iter().any(|n| n == "wx"));
    }
}

#[test]
fn startup_restrictions_disable_descriptors_and_deny_future_registration() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.register_runtime_tool(Arc::new(DummyTestTool::new("plugin_tool",))));

    let warnings = reg.apply_startup_tool_restrictions(&[
        "bash".to_string(),
        "plugin_tool".to_string(),
        "missing_tool".to_string(),
    ]);

    assert_eq!(warnings, vec!["missing_tool".to_string()]);
    assert!(
        reg.get("bash").is_some(),
        "disabled bundled-native tool remains registered"
    );
    assert!(
        reg.get("plugin_tool").is_some(),
        "disabled unloadable runtime tool remains registered"
    );
    assert!(
        !reg.definitions().iter().any(|d| d.name.as_ref() == "bash"),
        "startup-disabled bundled-native tool is hidden from model-visible definitions"
    );
    assert!(
        !reg.definitions()
            .iter()
            .any(|d| d.name.as_ref() == "plugin_tool"),
        "startup-disabled unloadable runtime tool is hidden from model-visible definitions"
    );
    let descriptor = reg.descriptor("bash").expect("descriptor");
    assert!(matches!(descriptor.source, ToolSource::BundledNative));
    assert!(!descriptor.availability.is_enabled());
    assert!(!reg.can_register_uds_tool_for_owner("bash", "uds:client:1"));
    assert!(!reg.can_register_uds_tool_for_owner("plugin_tool", "runtime:extension"));
    reg.unregister_runtime_tool("plugin_tool");
    assert!(
        reg.get("plugin_tool").is_none(),
        "owner-scoped unload remains a separate destructive lifecycle path"
    );
    assert!(
        !reg.register_runtime_tool(Arc::new(DummyTestTool::new("plugin_tool"))),
        "startup-disabled existing unloadable name stays denied after unload"
    );
    assert!(!reg.register_runtime_tool(Arc::new(DummyTestTool::new("missing_tool",))));
}

#[tokio::test]
async fn disabled_tool_execution_rejects_before_guards() {
    struct CountingGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl ToolGuard for CountingGuard {
        fn check(&self, _tool_name: &str, _arguments: &str) -> Result<(), String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err("guard should not run for disabled tools".to_string())
        }
    }

    let (mut reg, _tmp) = test_registry();
    let guard_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    reg.register_guard(Arc::new(CountingGuard(guard_calls.clone())));
    assert!(reg.disable_tool("ls"));

    let result = reg.execute("ls", r#"{}"#).await.expect("policy result");

    assert!(result.is_error, "disabled tool should reject execution");
    assert!(
        result.content.contains("disabled by runtime policy"),
        "unexpected disabled-tool message: {}",
        result.content
    );
    assert_eq!(
        guard_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "disabled policy should short-circuit before guards"
    );
}

#[test]
fn disable_then_disable_again_is_idempotent() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.disable_tool("read"));
    assert!(reg.disable_tool("read"));
    assert!(!reg.definitions().iter().any(|d| d.name.as_ref() == "read"));
    assert!(reg.enable_tool("read"));
    assert!(reg.enable_tool("read"));
}
