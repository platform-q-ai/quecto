//! #2150: what the event log records of each tool call.
use super::*;
use crate::application::audit::ports::AuditSink;
use crate::domain::audit::AuditEvent;

#[derive(Debug, Default)]
struct RecordingAudit {
    events: Mutex<Vec<AuditEvent>>,
}

impl AuditSink for RecordingAudit {
    fn emit(
        &self,
        _turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            self.events.lock().unwrap().push(event);
            Ok(())
        })
    }
}

/// A tool that takes a while, then answers ten bytes.
#[derive(Debug)]
struct SlowTool(ToolDefinition);

impl crate::application::tools::ports::Tool for SlowTool {
    fn definition(&self) -> ToolDefinition {
        self.0.clone()
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            Ok(ToolResult {
                content: "0123456789".into(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

/// Each tool result records how long the tool ran and the sizes of what it
/// was given and what it answered.
#[tokio::test]
async fn a_tool_result_records_its_duration_and_sizes() {
    let arguments = r#"{"q":"abc"}"#;
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("slow", arguments),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(SlowTool(ToolDefinition {
        name: "slow".into(),
        description: "slow mock".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
    })));
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        audit_log: Some(audit.clone()),
        ..test_config(provider, Box::new(registry))
    });
    agent
        .run_loop(&mut vec![Message::user("go")])
        .await
        .unwrap();
    let events = audit.events.lock().unwrap();
    let result = events
        .iter()
        .find_map(|event| match event {
            AuditEvent::ToolResult {
                tool,
                duration_ms,
                argument_bytes,
                content_bytes,
                ..
            } if tool == "slow" => Some((*duration_ms, *argument_bytes, *content_bytes)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no tool result: {events:?}"));
    assert!(result.0 >= 30, "{result:?}");
    assert_eq!(result.1, arguments.len());
    assert_eq!(result.2, 10);
}

/// #2150 review: when the model sent arguments the harness had to replace
/// (#2123), the tool call keeps what the model sent for diagnosis.
#[tokio::test]
async fn a_tool_call_keeps_the_models_raw_arguments_when_replaced() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("slow", "{not json"),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(SlowTool(ToolDefinition {
        name: "slow".into(),
        description: "slow mock".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
    })));
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        audit_log: Some(audit.clone()),
        ..test_config(provider, Box::new(registry))
    });
    agent
        .run_loop(&mut vec![Message::user("go")])
        .await
        .unwrap();
    let events = audit.events.lock().unwrap();
    let call = events
        .iter()
        .find_map(|event| match event {
            AuditEvent::ToolCall {
                arguments,
                raw_arguments,
                ..
            } => Some((arguments.clone(), raw_arguments.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no tool call: {events:?}"));
    assert_ne!(call.0, "{not json", "{call:?}");
    assert_eq!(call.1.as_deref(), Some("{not json"), "{call:?}");
}

/// A tool that fails with a long message whose cause is at its end.
#[derive(Debug)]
struct FailingTool;

impl crate::application::tools::ports::Tool for FailingTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "failing".into(),
            description: "fails".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        Box::pin(async {
            Ok(ToolResult {
                content: format!(
                    "Traceback:\n{}ValueError: the cause",
                    "  frame\n".repeat(100)
                ),
                is_error: true,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

/// #2159: a failed tool's logged preview keeps the end of its output, where
/// the cause is.
#[tokio::test]
async fn a_failed_tools_preview_keeps_its_cause() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("failing", "{}"),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(FailingTool));
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        audit_log: Some(audit.clone()),
        ..test_config(provider, Box::new(registry))
    });
    agent
        .run_loop(&mut vec![Message::user("go")])
        .await
        .unwrap();
    let events = audit.events.lock().unwrap();
    let preview = events
        .iter()
        .find_map(|event| match event {
            AuditEvent::ToolResult {
                tool,
                content_preview,
                ..
            } if tool == "failing" => Some(content_preview.clone()),
            _ => None,
        })
        .expect("the failed call is logged");
    assert!(preview.ends_with("ValueError: the cause"), "{preview}");
}
