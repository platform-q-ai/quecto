//! Contract tests for the `ToolExecutor` role port.
//!
//! Contract:
//! - `execute(name, args)` dispatches to the registered tool.
//! - Calling `execute` with an unknown name returns `Err` or an error result.

use quecto::domain::tool::{Tool, ToolDefinition, ToolExecutor, ToolResult};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct Echo {
    name: Cow<'static, str>,
}

impl Tool for Echo {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: Cow::Borrowed("echo"),
            parameters_schema: Cow::Borrowed(r#"{"type":"object"}"#),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ToolResult, quecto::domain::error::DomainError>> + Send + '_,
        >,
    > {
        let content = arguments.to_string();
        Box::pin(async move {
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

fn new_executor_with(tools: Vec<&'static str>) -> Arc<dyn ToolExecutor> {
    let mut reg = ToolRegistryImpl::new();
    for name in tools {
        reg.register(Arc::new(Echo {
            name: Cow::Borrowed(name),
        }));
    }
    Arc::new(reg)
}

#[tokio::test]
async fn execute_dispatches_to_the_registered_tool() {
    let executor = new_executor_with(vec!["alpha"]);
    let r = executor
        .execute("alpha", "hello")
        .await
        .expect("execute must not Err for a registered tool");
    assert!(!r.is_error);
    assert_eq!(
        r.content, "hello",
        "executor must pass the arguments through to the tool unchanged"
    );
}

#[tokio::test]
async fn empty_or_whitespace_arguments_are_normalized_to_empty_json_object() {
    let executor = new_executor_with(vec!["alpha"]);

    let empty = executor
        .execute("alpha", "")
        .await
        .expect("empty arguments must execute after normalization");
    let whitespace = executor
        .execute("alpha", "   ")
        .await
        .expect("whitespace arguments must execute after normalization");

    assert_eq!(empty.content, "{}");
    assert_eq!(whitespace.content, "{}");
}

#[tokio::test]
async fn execute_for_unknown_tool_signals_error() {
    let executor = new_executor_with(vec!["alpha"]);
    let signalled_error = match executor.execute("nonexistent", "").await {
        Err(_) => true,
        Ok(r) => r.is_error,
    };
    assert!(
        signalled_error,
        "executor must signal an error when the tool is not registered"
    );
}
