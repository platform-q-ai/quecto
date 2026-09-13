use super::*;

/// Minimal registry exercising the `ToolRegistry` default methods.
struct EmptyRegistry {
    defs: Vec<ToolDefinition>,
}

impl ToolCatalog for EmptyRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &self.defs
    }
}

impl ToolExecutor for EmptyRegistry {
    fn execute(
        &self,
        _name: &str,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(ToolResult {
                content: String::new(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

impl RuntimeToolLifecycleRegistry for EmptyRegistry {}

impl SessionAwareTools for EmptyRegistry {}

impl ToolPolicyMutator for EmptyRegistry {}

impl ToolRegistry for EmptyRegistry {}

fn def(name: &'static str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "".into(),
        parameters_schema: "{}".into(),
    }
}

#[test]
fn tool_count_defaults_to_definitions_len() {
    let reg = EmptyRegistry {
        defs: vec![def("a"), def("b")],
    };
    assert_eq!(reg.tool_count(), 2);
    assert_eq!(reg.tool_count(), reg.definitions().len());
}

#[tokio::test]
async fn extension_defaults_are_inert() {
    // Default ToolRegistry methods: no extension tracking; register/unregister no-op.
    let mut reg = EmptyRegistry { defs: vec![] };
    assert!(reg.runtime_tool_names().is_empty());
    reg.set_session_key("session-1"); // default no-op, must not panic
    reg.register_runtime_tool(std::sync::Arc::new(NoopTool)); // default no-op
    reg.unregister_runtime_tool("nope"); // no-op, must not panic
    assert!(reg.runtime_tool_names().is_empty());
    let result = reg.execute("missing", "{}").await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "");
}

/// Minimal `Tool` exercising the trait's default `set_session_key`.
struct NoopTool;

impl Tool for NoopTool {
    fn definition(&self) -> ToolDefinition {
        def("noop")
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(ToolResult {
                content: String::new(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

#[tokio::test]
async fn tool_default_set_session_key_is_inert() {
    let tool = NoopTool;
    tool.set_session_key("s".into()); // default no-op, must not panic
    assert_eq!(tool.definition().name, "noop");
    let result = tool.execute("{}").await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "");
}

#[test]
fn noop_tool_exercises_default_spawn_policy_snapshot_methods() {
    let tool = NoopTool;
    tool.set_inherited_child_policy_snapshot_for_spawn(std::collections::BTreeMap::new());
    assert!(tool.inherited_child_policy_snapshot_for_spawn().is_none());
}

#[test]
fn empty_registry_exercises_runtime_lifecycle_defaults() {
    let mut reg = EmptyRegistry { defs: vec![] };
    assert!(reg.unregister_runtime_tools_for_owner("owner").is_empty());
    assert!(!reg.register_uds_tool(std::sync::Arc::new(NoopTool)));
    assert_eq!(reg.extension_names(), Vec::<String>::new());
    assert!(!reg.register_extension(std::sync::Arc::new(NoopTool)));
    reg.unregister_extension("missing");
    assert!(reg.unregister_extensions_for_owner("owner").is_empty());
    assert!(!reg.register_uds_extension(std::sync::Arc::new(NoopTool)));
    assert!(reg.can_register_uds_tool_for_owner("tool", "owner"));
    assert!(reg.can_register_uds_tool_for_owner_with_stable_id("tool", "owner", Some("stable")));
    assert!(!reg.register_uds_tool_for_owner(
        std::sync::Arc::new(NoopTool),
        std::borrow::Cow::Borrowed("owner")
    ));
    assert!(!reg.register_uds_tool_for_owner_with_stable_id(
        std::sync::Arc::new(NoopTool),
        std::borrow::Cow::Borrowed("owner"),
        Some("stable".into())
    ));
    assert!(reg.can_register_uds_extension_for_owner("tool", "owner"));
    reg.set_inherited_child_policy_snapshot_for_spawn(std::collections::BTreeMap::new());
    assert!(reg.captured_spawn_snapshot().is_none());
    assert!(!reg.register_uds_extension_for_owner(
        std::sync::Arc::new(NoopTool),
        std::borrow::Cow::Borrowed("owner")
    ));
    assert!(!reg.enable_tool("missing"));
    assert!(!reg.disable_tool("missing"));
}
