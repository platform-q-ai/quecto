//! Contract tests for the `Extension` port.
//!
//! Contract:
//! - `name()` returns a non-empty identifier.
//! - `tools()` returns the same tools the extension was built with.
//! - `system_prompt_snippet()` is a pure accessor (no hidden side effects).

use quecto::domain::extension::Extension;
use quecto::domain::tool::{Tool, ToolDefinition, ToolResult};
use quecto::infrastructure::extensions::native::NativeExtension;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct NoopTool {
    n: Cow<'static, str>,
}
impl Tool for NoopTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.n.clone(),
            description: Cow::Borrowed("noop"),
            parameters_schema: Cow::Borrowed(r#"{"type":"object"}"#),
        }
    }
    fn execute(
        &self,
        _args: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ToolResult, quecto::domain::error::DomainError>> + Send + '_,
        >,
    > {
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

fn tool(name: &'static str) -> Arc<dyn Tool> {
    Arc::new(NoopTool {
        n: Cow::Borrowed(name),
    })
}

#[test]
fn name_is_exposed_verbatim() {
    let ext: Arc<dyn Extension> =
        Arc::new(NativeExtension::new("my-ext", "a description", tool("t")));
    assert_eq!(ext.name(), "my-ext");
    assert_eq!(ext.description(), "a description");
}

#[test]
fn tools_returns_all_tools_the_extension_was_built_with() {
    let ext: Arc<dyn Extension> = Arc::new(NativeExtension::with_tools(
        "multi",
        "many tools",
        vec![tool("a"), tool("b"), tool("c")],
    ));
    let names: Vec<_> = ext
        .tools()
        .iter()
        .map(|t| t.definition().name.to_string())
        .collect();
    assert_eq!(
        names,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}

#[test]
fn system_prompt_snippet_defaults_to_none() {
    // The default impl returns None; the contract permits Some(..) on
    // stateful extensions but here we assert the default behaviour.
    let ext: Arc<dyn Extension> = Arc::new(NativeExtension::new("n", "d", tool("t")));
    assert!(ext.system_prompt_snippet().is_none());
}
