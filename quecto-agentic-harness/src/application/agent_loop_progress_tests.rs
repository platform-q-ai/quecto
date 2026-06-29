use super::*;

#[tokio::test]
async fn test_progress_callback_tool_started_fired_for_each_tool_call() {
    let (agent, _, events) = make_agent_with_callback(
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
    let (agent, _, events) = make_agent_with_callback(
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
    let (agent, _, events) = make_agent_with_callback(
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
    let (agent, _, events) = make_agent_with_callback(
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

#[tokio::test]
async fn test_progress_callback_multiple_tool_calls_all_reported() {
    let (agent, _, events) = make_agent_with_callback(
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
    let (agent, _) = make_agent(vec![text_response("ok")], vec![]);
    let mut messages = vec![Message::user("hi")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "ok");
}

#[tokio::test]
async fn test_progress_callback_tool_started_includes_arguments() {
    let (agent, _, events) = make_agent_with_callback(
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
    let (agent, _, events) = make_agent_with_callback(
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
    let (agent, _, events) = make_agent_with_callback(
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

#[path = "agent_loop_retry_tests.rs"]
mod retry_tests;
