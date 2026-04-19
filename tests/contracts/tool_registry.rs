//! Contract tests for the `ToolRegistry` port.
//!
//! Contract:
//! - `definitions()` lists every registered tool.
//! - `tool_count()` matches `definitions().len()` for core tools.
//! - `execute(name, args)` dispatches to the registered tool.
//! - Calling `execute` with an unknown name returns `Err` (or an error
//!   result — whichever the registry chooses, the error must be surfaced).

use quecto::domain::tool::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A minimal real adapter implementing `Tool`, used to exercise the registry
/// through the port rather than depending on any specific production tool.
struct Echo { name: Cow<'static, str> }
impl Tool for Echo {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: Cow::Borrowed("echo"),
            parameters_schema: Cow::Borrowed(r#"{"type":"object"}"#),
        }
    }
    fn execute(&self, arguments: &str) -> Pin<Box<dyn Future<Output = Result<ToolResult, quecto::domain::error::DomainError>> + Send + '_>> {
        let content = arguments.to_string();
        Box::pin(async move {
            Ok(ToolResult { content, is_error: false, image_blocks: vec![] })
        })
    }
}

fn new_registry_with(tools: Vec<(&'static str,)>) -> Arc<dyn ToolRegistry> {
    let mut reg = ToolRegistryImpl::new();
    for (n,) in tools {
        reg.register(Arc::new(Echo { name: Cow::Borrowed(n) }));
    }
    Arc::new(reg)
}

#[test]
fn empty_registry_has_no_definitions() {
    let reg = new_registry_with(vec![]);
    assert_eq!(reg.definitions().len(), 0);
    assert_eq!(reg.tool_count(), 0);
}

#[test]
fn definitions_cover_every_registered_tool() {
    let reg = new_registry_with(vec![("alpha",), ("beta",), ("gamma",)]);
    let names: Vec<_> = reg.definitions().iter().map(|d| d.name.as_ref()).collect();
    assert_eq!(reg.tool_count(), 3);
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"gamma"));
}

#[tokio::test]
async fn execute_dispatches_to_the_registered_tool() {
    let reg = new_registry_with(vec![("alpha",)]);
    let r = reg.execute("alpha", "hello").await
        .expect("execute must not Err for a registered tool");
    assert!(!r.is_error);
    assert_eq!(r.content, "hello",
        "registry must pass the arguments through to the tool unchanged");
}

#[tokio::test]
async fn execute_for_unknown_tool_signals_error() {
    let reg = new_registry_with(vec![("alpha",)]);
    let signalled_error = match reg.execute("nonexistent", "").await {
        Err(_) => true,
        Ok(r) => r.is_error,
    };
    assert!(signalled_error,
        "registry must signal an error when the tool is not registered");
}
