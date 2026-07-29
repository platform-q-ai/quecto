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
            })
        })
    }
}

impl ExtensionToolRegistry for EmptyRegistry {}

impl SessionAwareTools for EmptyRegistry {}

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
    assert!(reg.extension_names().is_empty());
    reg.set_session_key("session-1"); // default no-op, must not panic
    reg.register_extension(std::sync::Arc::new(NoopTool)); // default no-op
    reg.unregister_extension("nope"); // no-op, must not panic
    assert!(reg.extension_names().is_empty());
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
fn tool_result_and_image_block_construct() {
    let r = ToolResult {
        content: "ok".into(),
        is_error: false,
        image_blocks: vec![ImageBlock {
            mime_type: "image/png",
            data: "AAAA".into(),
        }],
    };
    assert!(!r.is_error);
    assert_eq!(r.image_blocks[0].mime_type, "image/png");
}
