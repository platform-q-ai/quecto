use super::*;
use crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

#[tokio::test]
async fn test_progress_callback_tool_started_fired_for_each_tool_call() {
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "hi")],
    );
    let mut messages = vec![Message::user("run echo")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let tool_started_count = fired
        .iter()
        .filter(|e| matches!(e, crate::domain::agent::AgentProgressEvent::ToolStarted { name, .. } if name == "bash"))
        .count();
    assert_eq!(
        tool_started_count, 1,
        "expected 1 ToolStarted(bash) event, got: {:?}",
        *fired
    );
}

#[tokio::test]
async fn test_progress_callback_tool_finished_fired_after_tool_executes() {
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "hi")],
    );
    let mut messages = vec![Message::user("run echo")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let tool_finished = fired
        .iter()
        .find(|e| matches!(e, crate::domain::agent::AgentProgressEvent::ToolFinished { name, .. } if name == "bash"));
    assert!(
        tool_finished.is_some(),
        "expected ToolFinished(bash) event, got: {:?}",
        *fired
    );
}

#[tokio::test]
async fn test_progress_callback_event_order_thinking_tool_started_tool_finished_done() {
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "hi")],
    );
    let mut messages = vec![Message::user("run echo")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();

    // Find positions of key event types
    let thinking_pos = fired
        .iter()
        .position(|e| matches!(e, crate::domain::agent::AgentProgressEvent::Thinking { .. }));
    let tool_started_pos = fired.iter().position(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolStarted { .. }
        )
    });
    let tool_finished_pos = fired.iter().position(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolFinished { .. }
        )
    });
    let done_pos = fired
        .iter()
        .rposition(|e| matches!(e, crate::domain::agent::AgentProgressEvent::Done));

    assert!(thinking_pos.is_some(), "expected Thinking event");
    assert!(tool_started_pos.is_some(), "expected ToolStarted event");
    assert!(tool_finished_pos.is_some(), "expected ToolFinished event");
    assert!(done_pos.is_some(), "expected Done event");

    let t = thinking_pos.unwrap();
    let ts = tool_started_pos.unwrap();
    let tf = tool_finished_pos.unwrap();
    let d = done_pos.unwrap();

    assert!(t < ts, "Thinking should fire before ToolStarted");
    assert!(ts < tf, "ToolStarted should fire before ToolFinished");
    assert!(tf < d, "ToolFinished should fire before Done");
}

#[tokio::test]
async fn test_progress_callback_tool_finished_captures_duration_and_error_flag() {
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "output")],
    );
    let mut messages = vec![Message::user("run it")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    if let Some(crate::domain::agent::AgentProgressEvent::ToolFinished {
        name,
        arguments,
        duration_ms,
        is_error,
        ..
    }) = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolFinished { .. }
        )
    }) {
        assert_eq!(name, "bash");
        assert!(
            arguments.contains("echo hi"),
            "expected ToolFinished arguments to include command, got: {arguments}"
        );
        // duration_ms may be 0 in test environments, but must not panic
        let _ = *duration_ms;
        assert!(!is_error, "mock tool should not be an error");
    } else {
        panic!("expected ToolFinished event, got: {:?}", *fired);
    }
}

/// #1317: `Ok(ToolResult { is_error: true })` must reach model-facing tool
/// messages and ToolFinished progress (not only `Err(DomainError)`).
#[tokio::test]
async fn ok_tool_result_is_error_propagates_to_message_and_progress() {
    use crate::domain::audit::{AuditEvent, AuditSink};
    use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
    use std::pin::Pin;

    #[derive(Debug)]
    struct ErrorTool {
        def: ToolDefinition,
        content: String,
    }

    impl Tool for ErrorTool {
        fn definition(&self) -> ToolDefinition {
            self.def.clone()
        }

        fn execute(
            &self,
            _arguments: &str,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
        {
            let content = self.content.clone();
            Box::pin(async move {
                Ok(ToolResult {
                    content,
                    is_error: true,
                    image_blocks: vec![],
                })
            })
        }
    }

    #[derive(Debug, Default)]
    struct RecordingAudit {
        events: Mutex<Vec<AuditEvent>>,
    }

    impl AuditSink for RecordingAudit {
        fn emit(
            &self,
            _turn: u32,
            event: AuditEvent,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>>
        {
            Box::pin(async move {
                self.events.lock().unwrap().push(event);
                Ok(())
            })
        }
    }

    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("bash", r#"{"command":"bad"}"#),
        text_response("acknowledged"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(ErrorTool {
        def: ToolDefinition {
            name: "bash".into(),
            description: "error mock".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        },
        content: "llm-addressable failure".into(),
    }));
    let events: Arc<Mutex<Vec<crate::domain::agent::AgentProgressEvent>>> =
        Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let callback: crate::domain::agent::ProgressCallback =
        Arc::new(move |ev| events_clone.lock().unwrap().push(ev));
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        progress_callback: Some(callback),
        audit_log: Some(audit.clone()),
        ..test_config(provider, Box::new(registry))
    });

    let mut messages = vec![Message::user("run it")];
    agent.run_loop(&mut messages).await.unwrap();

    let tool_msg = messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("expected tool message in conversation");
    assert!(
        tool_msg.is_error,
        "tool Message.is_error must be true for Ok(is_error: true), got false"
    );
    assert_eq!(tool_msg.content, "llm-addressable failure");

    let fired = events.lock().unwrap();
    let finished = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolFinished { name, .. } if name == "bash"
        )
    });
    match finished {
        Some(crate::domain::agent::AgentProgressEvent::ToolFinished { is_error, .. }) => {
            assert!(
                *is_error,
                "ToolFinished.is_error must be true for Ok(is_error: true)"
            );
        }
        other => panic!(
            "expected ToolFinished(bash), got: {other:?} (all: {:?})",
            *fired
        ),
    }

    let audit_events = audit.events.lock().unwrap();
    assert!(
        audit_events.iter().any(|e| matches!(
            e,
            AuditEvent::ToolResult { tool, is_error: true, .. } if tool == "bash"
        )),
        "expected AuditEvent::ToolResult is_error=true, got: {audit_events:?}"
    );
}

#[tokio::test]
async fn test_progress_callback_multiple_tool_calls_all_reported() {
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("read", r#"{"path":"a.txt"}"#),
            tool_call_response("write", r#"{"path":"b.txt","content":"x"}"#),
            text_response("done"),
        ],
        vec![("read", "content"), ("write", "ok")],
    );
    let mut messages = vec![Message::user("copy a to b")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let started: Vec<&str> = fired
        .iter()
        .filter_map(|e| {
            if let crate::domain::agent::AgentProgressEvent::ToolStarted { name, .. } = e {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(
        started.contains(&"read"),
        "expected ToolStarted(read), got: {:?}",
        started
    );
    assert!(
        started.contains(&"write"),
        "expected ToolStarted(write), got: {:?}",
        started
    );
}

#[tokio::test]
async fn test_progress_callback_none_does_not_panic() {
    // Verify that having no callback at all does not change behaviour
    let (mut agent, _) = make_agent(vec![text_response("ok")], vec![]);
    let mut messages = vec![Message::user("hi")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "ok");
}

#[tokio::test]
async fn test_progress_callback_tool_started_includes_arguments() {
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hello world"}"#),
            text_response("done"),
        ],
        vec![("bash", "hello world")],
    );
    let mut messages = vec![Message::user("run it")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    if let Some(crate::domain::agent::AgentProgressEvent::ToolStarted {
        name, arguments, ..
    }) = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolStarted { .. }
        )
    }) {
        assert_eq!(name, "bash");
        // arguments should be the raw JSON — not truncated at the domain level
        assert!(!arguments.is_empty(), "arguments should not be empty");
        assert!(
            arguments.contains("echo hello world"),
            "arguments should contain the command, got: {arguments}"
        );
    } else {
        panic!("expected ToolStarted event, got: {:?}", *fired);
    }
}

// --- #214: tool_count() on ToolRegistry trait ---

#[tokio::test]
async fn test_tool_count_on_registry_trait() {
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("bash", "")));
    registry.register(Arc::new(MockTool::new("read", "")));
    let trait_reg: &dyn ToolRegistry = &registry;
    assert_eq!(trait_reg.tool_count(), 2);
}

#[tokio::test]
async fn test_tool_count_empty() {
    let registry = MockRegistry::new();
    let trait_reg: &dyn ToolRegistry = &registry;
    assert_eq!(trait_reg.tool_count(), 0);
}

// --- #318: tool_call_id in ToolStarted/ToolFinished progress events ---

#[tokio::test]
async fn test_progress_callback_tool_started_includes_tool_call_id() {
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "hi")],
    );
    let mut messages = vec![Message::user("run echo")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    if let Some(crate::domain::agent::AgentProgressEvent::ToolStarted {
        tool_call_id, name, ..
    }) = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolStarted { .. }
        )
    }) {
        assert_eq!(name, "bash");
        assert_eq!(
            tool_call_id, "call_bash",
            "expected tool_call_id 'call_bash', got '{tool_call_id}'"
        );
    } else {
        panic!("expected ToolStarted event, got: {:?}", *fired);
    }
}

#[tokio::test]
async fn test_progress_callback_tool_finished_includes_tool_call_id() {
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "hi")],
    );
    let mut messages = vec![Message::user("run echo")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    if let Some(crate::domain::agent::AgentProgressEvent::ToolFinished {
        tool_call_id, name, ..
    }) = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolFinished { .. }
        )
    }) {
        assert_eq!(name, "bash");
        assert_eq!(
            tool_call_id, "call_bash",
            "expected tool_call_id 'call_bash', got '{tool_call_id}'"
        );
    } else {
        panic!("expected ToolFinished event, got: {:?}", *fired);
    }
}

#[tokio::test]
async fn test_progress_callback_tool_finished_preview_handles_mid_codepoint_cap() {
    let multibyte = "€".repeat(DEFAULT_OUTPUT_CAP_BYTES / "€".len() + 1);
    let (mut agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"emit multibyte"}"#),
            text_response("done"),
        ],
        vec![("bash", &multibyte)],
    );
    let mut messages = vec![Message::user("run it")];

    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let result_content = fired
        .iter()
        .find_map(|e| {
            if let crate::domain::agent::AgentProgressEvent::ToolFinished {
                result_content, ..
            } = e
            {
                Some(result_content)
            } else {
                None
            }
        })
        .expect("expected ToolFinished event");
    let expected_chars = DEFAULT_OUTPUT_CAP_BYTES / "€".len();
    let expected = "€".repeat(expected_chars);
    assert_eq!(
        result_content, &expected,
        "preview should keep the maximal valid UTF-8 prefix within the byte cap"
    );
    assert_eq!(
        result_content.len(),
        expected_chars * "€".len(),
        "preview should stay within byte cap without dropping valid complete characters"
    );
}

#[path = "agent_loop_retry_tests.rs"]
mod retry_tests;
