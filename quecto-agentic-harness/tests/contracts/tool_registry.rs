//! Contract tests for the composed `ToolRegistry` bundle port.
//!
//! Role-specific contracts live in `tool_catalog`, `tool_executor`,
//! `runtime_tool_lifecycle_registry`, and `session_aware_tools`. This module proves
//! the composition-root bundle still exposes the combined capability expected by
//! `AgentLoopImpl`.

use quecto::domain::tool::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A minimal real adapter implementing `Tool`, used to exercise the registry
/// through the port rather than depending on any specific production tool.
struct Echo {
    name: Cow<'static, str>,
    seen_sessions: Option<Arc<std::sync::Mutex<Vec<String>>>>,
}
impl Tool for Echo {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: Cow::Borrowed("echo"),
            parameters_schema: Cow::Borrowed(r#"{"type":"object"}"#),
        }
    }

    fn set_session_key(&self, session_key: String) {
        if let Some(seen) = &self.seen_sessions {
            seen.lock().unwrap().push(session_key);
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

fn new_registry_with(tools: Vec<(&'static str,)>) -> Arc<dyn ToolRegistry> {
    let mut reg = ToolRegistryImpl::new();
    for (n,) in tools {
        reg.register(Arc::new(Echo {
            name: Cow::Borrowed(n),
            seen_sessions: None,
        }));
    }
    Arc::new(reg)
}

#[test]
fn full_registry_bundle_exposes_extension_and_session_roles() {
    let seen_sessions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut reg: Box<dyn ToolRegistry> = Box::new(ToolRegistryImpl::new());

    reg.register_runtime_tool(Arc::new(Echo {
        name: Cow::Borrowed("extension"),
        seen_sessions: Some(seen_sessions.clone()),
    }));
    assert_eq!(reg.runtime_tool_names(), vec!["extension"]);

    reg.set_session_key("session-a");
    assert_eq!(
        *seen_sessions.lock().unwrap(),
        vec!["session-a".to_string()]
    );

    reg.unregister_runtime_tool("extension");
    assert!(reg.runtime_tool_names().is_empty());
    assert_eq!(reg.tool_count(), 0);
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
    let r = reg
        .execute("alpha", "hello")
        .await
        .expect("execute must not Err for a registered tool");
    assert!(!r.is_error);
    assert_eq!(
        r.content, "hello",
        "registry must pass the arguments through to the tool unchanged"
    );
}

#[tokio::test]
async fn execute_for_unknown_tool_signals_error() {
    let reg = new_registry_with(vec![("alpha",)]);
    let signalled_error = match reg.execute("nonexistent", "").await {
        Err(_) => true,
        Ok(r) => r.is_error,
    };
    assert!(
        signalled_error,
        "registry must signal an error when the tool is not registered"
    );
}
