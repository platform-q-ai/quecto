//! Contract tests for the `SessionAwareTools` role port.
//!
//! Contract:
//! - session-key changes are propagated to every registered tool.

use quecto::domain::tool::{SessionAwareTools, Tool, ToolDefinition, ToolResult};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

struct SessionRecordingTool {
    name: &'static str,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Tool for SessionRecordingTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: Cow::Borrowed(self.name),
            description: Cow::Borrowed("records session keys"),
            parameters_schema: Cow::Borrowed(r#"{"type":"object"}"#),
        }
    }

    fn set_session_key(&self, session_key: String) {
        self.seen.lock().unwrap().push(session_key);
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ToolResult, quecto::domain::error::DomainError>> + Send + '_,
        >,
    > {
        Box::pin(async move {
            Ok(ToolResult {
                content: String::new(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

#[test]
fn set_session_key_reaches_registered_tools() {
    let first_seen = Arc::new(Mutex::new(Vec::new()));
    let second_seen = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(SessionRecordingTool {
        name: "first",
        seen: first_seen.clone(),
    }));
    registry.register_runtime_tool(Arc::new(SessionRecordingTool {
        name: "second",
        seen: second_seen.clone(),
    }));

    let session_tools: &dyn SessionAwareTools = &registry;
    session_tools.set_session_key("session-a");
    session_tools.set_session_key("session-b");

    let expected = vec!["session-a".to_string(), "session-b".to_string()];
    assert_eq!(*first_seen.lock().unwrap(), expected);
    assert_eq!(*second_seen.lock().unwrap(), expected);
}
