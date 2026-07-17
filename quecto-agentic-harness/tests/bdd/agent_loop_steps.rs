use super::*;
use quecto::application::agent_loop_test_support;
use quecto::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

// Agent Loop Steps
// ===========================================================================

/// Helper: ensure a mock LLM provider is created and a basic agent loop
/// can be built. Returns the mock provider (for queuing responses).
pub(super) fn ensure_mock_llm(world: &mut QuectoWorld) -> Arc<MockLlmProvider> {
    if world.mock_llm.is_none() {
        world.mock_llm = Some(Arc::new(MockLlmProvider::new()));
    }
    world.mock_llm.clone().unwrap()
}

/// Helper: build an AgentLoopImpl from the world's current state.
fn build_agent_loop(world: &QuectoWorld, max_iterations: Option<u32>) -> AgentLoopImpl {
    build_agent_loop_with_callback(world, max_iterations, None)
}

fn build_agent_loop_with_callback(
    world: &QuectoWorld,
    max_iterations: Option<u32>,
    progress_callback: Option<Arc<dyn Fn(quecto::domain::agent::AgentProgressEvent) + Send + Sync>>,
) -> AgentLoopImpl {
    let provider = world.mock_llm.clone().expect("mock LLM not configured") as Arc<dyn LlmProvider>;

    // Build a tool registry from mock_tools or tool_registry
    let registry = if !world.mock_tools.is_empty() {
        let mut reg = ToolRegistryImpl::new();
        for tool in world.mock_tools.values() {
            reg.register(tool.clone());
        }
        reg
    } else if let Some(ref reg) = world.tool_registry {
        // We can't clone ToolRegistryImpl, so build a new empty one for scenarios
        // that don't need tools.
        let _ = reg;
        ToolRegistryImpl::new()
    } else {
        ToolRegistryImpl::new()
    };

    let mut agent = AgentLoopImpl::new(quecto::application::agent_loop::AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    });

    if let Some(max) = max_iterations {
        agent = agent.with_max_tool_iterations(max);
    }

    agent
}

#[given("a configured agent with a mock LLM")]
fn given_configured_agent_with_mock(world: &mut QuectoWorld) {
    ensure_mock_llm(world);
}

#[given(expr = "the LLM returns a plain text response {string}")]
fn given_llm_returns_text(world: &mut QuectoWorld, text: String) {
    let mock = ensure_mock_llm(world);
    mock.push_response(LlmResponse {
        content: Some(text),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });
}

#[given(expr = "the LLM returns a tool call for {string} with args:")]
fn given_llm_returns_tool_call(world: &mut QuectoWorld, tool_name: String, step: &gherkin::Step) {
    let mock = ensure_mock_llm(world);
    let table = step.table.as_ref().expect("step should have a table");
    let args_json = table_to_json(table);
    mock.push_response(LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{}", tool_name),
            name: tool_name,
            arguments: args_json,
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });
}

#[given(expr = "the tool {string} returns {string}")]
fn given_tool_returns(world: &mut QuectoWorld, tool_name: String, response: String) {
    let tool = Arc::new(MockBddTool::new(&tool_name, &response));
    world.mock_tools.insert(tool_name, tool);
}

#[given(expr = "the tool {string} returns long Unicode output")]
fn given_tool_returns_mid_codepoint_output(world: &mut QuectoWorld, tool_name: String) {
    let output = "€".repeat(DEFAULT_OUTPUT_CAP_BYTES / "€".len() + 1);
    let tool = Arc::new(MockBddTool::new(&tool_name, &output));
    world.mock_tools.insert(tool_name, tool);
}

#[given(expr = "the tool {string} returns output {word} the allowed display length")]
fn given_tool_returns_output_at_display_boundary(
    world: &mut QuectoWorld,
    tool_name: String,
    position: String,
) {
    let length = match position.as_str() {
        "below" => DEFAULT_OUTPUT_CAP_BYTES - 1,
        "above" => DEFAULT_OUTPUT_CAP_BYTES + 1,
        other => panic!("unsupported boundary position: {other}"),
    };
    let output = "a".repeat(length);
    let tool = Arc::new(MockBddTool::new(&tool_name, &output));
    world.mock_tools.insert(tool_name, tool);
}

#[given(expr = "the tool {string} returns output exactly at the allowed display length")]
fn given_tool_returns_output_exactly_at_display_boundary(
    world: &mut QuectoWorld,
    tool_name: String,
) {
    let output = "a".repeat(DEFAULT_OUTPUT_CAP_BYTES);
    let tool = Arc::new(MockBddTool::new(&tool_name, &output));
    world.mock_tools.insert(tool_name, tool);
}

#[given(expr = "the LLM then returns {string}")]
fn given_llm_then_returns(world: &mut QuectoWorld, text: String) {
    let mock = ensure_mock_llm(world);
    mock.push_response(LlmResponse {
        content: Some(text),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });
}

#[given(expr = "the LLM returns simultaneous tool calls for {string} and {string}")]
fn given_llm_returns_simultaneous_tool_calls(
    world: &mut QuectoWorld,
    tool1: String,
    tool2: String,
) {
    let mock = ensure_mock_llm(world);
    world.executed_tools.lock().unwrap().clear();
    mock.push_response(LlmResponse {
        content: None,
        tool_calls: vec![
            ToolCall {
                id: format!("call_{}", tool1),
                name: tool1.clone(),
                arguments: r#"{"path":"notes.txt"}"#.to_string(),
            },
            ToolCall {
                id: format!("call_{}", tool2),
                name: tool2.clone(),
                arguments: r#"{"path":"out.txt","content":"copied"}"#.to_string(),
            },
        ],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });

    for tool_name in [&tool1, &tool2] {
        if !world.mock_tools.contains_key(tool_name) {
            let tool =
                MockBddTool::new(tool_name, "ok").with_calls(Arc::clone(&world.executed_tools));
            world.mock_tools.insert(tool_name.clone(), Arc::new(tool));
        }
    }
}

#[given(expr = "the LLM returns sequential tool calls for {string} then {string}")]
fn given_llm_returns_sequential_tool_calls(world: &mut QuectoWorld, tool1: String, tool2: String) {
    let mock = ensure_mock_llm(world);
    world.executed_tools.lock().unwrap().clear();
    for tool_name in [&tool1, &tool2] {
        mock.push_response(LlmResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: format!("call_{}", tool_name),
                name: tool_name.to_string(),
                arguments: "{}".to_string(),
            }],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        });
        if !world.mock_tools.contains_key(tool_name.as_str()) {
            let tool =
                MockBddTool::new(tool_name, "ok").with_calls(Arc::clone(&world.executed_tools));
            world.mock_tools.insert(tool_name.clone(), Arc::new(tool));
        }
    }
}

#[given(expr = "a configured agent with max_tool_iterations {int}")]
fn given_agent_with_max_iterations(world: &mut QuectoWorld, max: u32) {
    ensure_mock_llm(world);
    // Store max iterations; will be used when building the agent
    world
        .env_overrides
        .insert("_max_tool_iterations".to_string(), max.to_string());
}

#[given("the LLM always returns a tool call")]
fn given_llm_always_returns_tool_call(world: &mut QuectoWorld) {
    let mock = ensure_mock_llm(world);
    // Queue many tool call responses (more than any reasonable limit)
    for i in 0..50 {
        mock.push_response(LlmResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: format!("call_{}", i),
                name: "bash".to_string(),
                arguments: r#"{"command":"echo hi"}"#.to_string(),
            }],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        });
    }
    // Register the exec mock tool
    if !world.mock_tools.contains_key("bash") {
        world.mock_tools.insert(
            "bash".to_string(),
            Arc::new(MockBddTool::new("bash", "output")),
        );
    }
}

#[given(expr = "a configured agent with tools {string} and {string}")]
fn given_agent_with_tools(world: &mut QuectoWorld, tool1: String, tool2: String) {
    ensure_mock_llm(world);
    world
        .mock_tools
        .insert(tool1.clone(), Arc::new(MockBddTool::new(&tool1, "")));
    world
        .mock_tools
        .insert(tool2.clone(), Arc::new(MockBddTool::new(&tool2, "")));
}

#[given("a fully initialized agent")]
fn given_fully_initialized_agent(world: &mut QuectoWorld) {
    ensure_mock_llm(world);
    // Register some tools to have a non-zero count
    world
        .mock_tools
        .insert("bash".to_string(), Arc::new(MockBddTool::new("bash", "")));
    world
        .mock_tools
        .insert("read".to_string(), Arc::new(MockBddTool::new("read", "")));
    world
        .mock_tools
        .insert("write".to_string(), Arc::new(MockBddTool::new("write", "")));
}

#[when(expr = "the agent processes message {string}")]
fn when_agent_processes_message(world: &mut QuectoWorld, message: String) {
    let max_iter = world
        .env_overrides
        .get("_max_tool_iterations")
        .and_then(|v| v.parse::<u32>().ok());
    let agent = build_agent_loop(world, max_iter);

    let mut messages = vec![Message::user(message)];

    let preview_sentinel = "€".repeat(DEFAULT_OUTPUT_CAP_BYTES / "€".len() + 1);
    agent_loop_test_support::reset_built_tool_result_preview_count_for_tests(&preview_sentinel);
    agent_loop_test_support::reset_tool_call_clone_count_for_tests("call_read");
    agent_loop_test_support::reset_tool_call_clone_count_for_tests("call_write");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages));

    world.agent_result = Some(result.expect("agent process failed"));
    world.tool_result_preview_build_count =
        Some(agent_loop_test_support::built_tool_result_preview_count_for_tests(&preview_sentinel));
    world.headless_tool_call_clone_count = Some(
        agent_loop_test_support::tool_call_clone_count_for_tests("call_read")
            + agent_loop_test_support::tool_call_clone_count_for_tests("call_write"),
    );
    world.headless_tool_calls_issued =
        Some(messages.iter().map(|m| m.tool_calls.len()).sum::<usize>());
}

#[when(expr = "the agent reports progress while processing message {string}")]
fn when_agent_processes_message_with_progress(world: &mut QuectoWorld, message: String) {
    let preview = Arc::new(std::sync::Mutex::new(None));
    let preview_for_callback = Arc::clone(&preview);
    let completed_roles = Arc::new(std::sync::Mutex::new(Vec::new()));
    let completed_roles_for_callback = Arc::clone(&completed_roles);
    let callback = Arc::new(move |event| match event {
        quecto::domain::agent::AgentProgressEvent::ToolFinished { result_content, .. } => {
            *preview_for_callback.lock().unwrap() = Some(result_content);
        }
        quecto::domain::agent::AgentProgressEvent::TurnCompleted { messages } => {
            *completed_roles_for_callback.lock().unwrap() = messages
                .iter()
                .map(|message| message.role.clone())
                .collect();
        }
        _ => {}
    });
    let max_iter = world
        .env_overrides
        .get("_max_tool_iterations")
        .and_then(|v| v.parse::<u32>().ok());
    let agent = build_agent_loop_with_callback(world, max_iter, Some(callback));

    let mut messages = vec![Message::user(message)];
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages));

    world.agent_result = Some(result.expect("agent process failed"));
    world.tool_result_preview = preview.lock().unwrap().clone();
    world.completed_turn_roles = completed_roles.lock().unwrap().clone();
}

#[when("the agent sends a request to the LLM")]
fn when_agent_sends_request(world: &mut QuectoWorld) {
    let agent = build_agent_loop(world, None);

    // Queue a simple text response so the loop completes
    let mock = world.mock_llm.as_ref().unwrap();
    mock.push_response(LlmResponse {
        content: Some("ok".to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });

    let mut messages = vec![Message::user("test")];

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages))
        .expect("agent process failed while capturing tool definitions");
}

#[when("I query the startup info")]
fn when_query_startup_info(world: &mut QuectoWorld) {
    let agent = build_agent_loop(world, None);
    world.agent_info = Some(agent.info());
}

#[then(expr = "the response should be {string}")]
fn then_response_should_be(world: &mut QuectoWorld, expected: String) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert_eq!(
        result.response, expected,
        "expected response '{}', got '{}'",
        expected, result.response
    );
}

#[then("both tools should be executed in order")]
fn then_both_tools_executed(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert_eq!(
        result.tool_iterations, 1,
        "expected one LLM tool turn, got {}",
        result.tool_iterations
    );
    assert_eq!(
        world.executed_tools.lock().unwrap().as_slice(),
        &["read".to_string(), "write".to_string()],
        "tool calls should execute in assistant-request order"
    );
}

#[then("both tools should be executed across two turns in order")]
fn then_both_tools_executed_sequentially(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert_eq!(
        result.tool_iterations, 2,
        "expected two sequential LLM tool turns, got {}",
        result.tool_iterations
    );
    assert_eq!(
        world.executed_tools.lock().unwrap().as_slice(),
        &["read".to_string(), "write".to_string()],
        "tool calls should execute in turn order"
    );
}

#[then("the tool result preview should contain only complete characters")]
fn then_tool_result_preview_contains_complete_characters(world: &mut QuectoWorld) {
    let preview = world
        .tool_result_preview
        .as_ref()
        .expect("tool result preview not captured");
    assert!(
        preview.chars().all(|ch| ch == '€'),
        "preview must contain only complete multibyte characters, got {preview:?}"
    );
    assert!(
        !preview.is_empty(),
        "preview should retain complete characters"
    );
}

#[then("the tool result preview should stay within the allowed display length")]
fn then_tool_result_preview_stays_within_byte_limit(world: &mut QuectoWorld) {
    let preview = world
        .tool_result_preview
        .as_ref()
        .expect("tool result preview not captured");
    let expected_chars = DEFAULT_OUTPUT_CAP_BYTES / "€".len();
    assert_eq!(
        preview,
        &"€".repeat(expected_chars),
        "preview should keep the maximal valid UTF-8 prefix within the byte limit"
    );
}

#[then(expr = "the tool result preview should match output {word} the allowed display length")]
fn then_preview_matches_output_at_display_boundary(world: &mut QuectoWorld, position: String) {
    let preview = world
        .tool_result_preview
        .as_ref()
        .expect("tool result preview not captured");
    let expected_length = match position.as_str() {
        "below" => DEFAULT_OUTPUT_CAP_BYTES - 1,
        "above" => DEFAULT_OUTPUT_CAP_BYTES,
        other => panic!("unsupported boundary position: {other}"),
    };
    assert_eq!(preview, &"a".repeat(expected_length));
}

#[then("the tool result preview should match output exactly at the allowed display length")]
fn then_preview_matches_output_exactly_at_display_boundary(world: &mut QuectoWorld) {
    let preview = world
        .tool_result_preview
        .as_ref()
        .expect("tool result preview not captured");
    assert_eq!(preview, &"a".repeat(DEFAULT_OUTPUT_CAP_BYTES));
}

#[then("no progress preview should be reported")]
fn then_no_tool_result_preview_should_be_captured(world: &mut QuectoWorld) {
    assert_eq!(
        world.tool_result_preview_build_count,
        Some(0),
        "headless processing should not build tool result previews"
    );
}

#[then(
    "execution should borrow the tool requests and the run ledger should retain one copy per request"
)]
fn then_tool_requests_borrowed_and_ledger_retains_one_copy_each(world: &mut QuectoWorld) {
    let issued = world
        .headless_tool_calls_issued
        .expect("no headless turn was processed");
    assert!(issued > 0, "scenario setup: the turn must issue tool calls");
    assert_eq!(
        world.headless_tool_call_clone_count,
        Some(issued),
        "execution borrows tool requests; exactly one clone per issued \
         request is retained by the run append ledger (#1072)"
    );
}

#[then("the completed turn should include both tool results in order")]
fn then_completed_turn_should_include_both_tool_results_in_order(world: &mut QuectoWorld) {
    assert_eq!(
        world.completed_turn_roles,
        vec![Role::Assistant, Role::Tool, Role::Tool],
        "completed turn should include assistant request followed by both tool results"
    );
}

#[then("the final response should confirm completion")]
fn then_final_response_confirms_completion(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert!(
        !result.response.is_empty(),
        "expected a non-empty final response"
    );
    assert!(
        !result.iteration_limit_reached,
        "should not have hit iteration limit"
    );
}

#[then(expr = "the agent should stop after {int} tool iterations")]
fn then_agent_stops_after_iterations(world: &mut QuectoWorld, expected: u32) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert_eq!(
        result.tool_iterations, expected,
        "expected {} tool iterations, got {}",
        expected, result.tool_iterations
    );
}

#[then("the response should indicate the iteration limit was reached")]
fn then_response_indicates_limit(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert!(
        result.iteration_limit_reached,
        "expected iteration_limit_reached to be true"
    );
    assert!(
        result.response.contains("limit"),
        "expected response to mention 'limit', got: {}",
        result.response
    );
}

#[then(expr = "the request should include tool definitions for {string} and {string}")]
fn then_request_includes_tool_defs(world: &mut QuectoWorld, tool1: String, tool2: String) {
    let mock = world.mock_llm.as_ref().expect("no mock LLM");
    let defs = mock.last_tool_defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_ref()).collect();
    assert!(
        names.contains(&tool1.as_str()),
        "expected tool definitions to include '{}', got: {:?}",
        tool1,
        names
    );
    assert!(
        names.contains(&tool2.as_str()),
        "expected tool definitions to include '{}', got: {:?}",
        tool2,
        names
    );
}

#[then("each tool definition should have name, description, and parameters")]
fn then_each_tool_def_has_fields(world: &mut QuectoWorld) {
    let mock = world.mock_llm.as_ref().expect("no mock LLM");
    let defs = mock.last_tool_defs();
    assert!(!defs.is_empty(), "expected at least one tool definition");
    for def in &defs {
        assert!(!def.name.is_empty(), "tool name should not be empty");
        assert!(
            !def.description.is_empty(),
            "tool '{}' description should not be empty",
            def.name
        );
        assert!(
            !def.parameters_schema.is_empty(),
            "tool '{}' parameters_schema should not be empty",
            def.name
        );
    }
}

#[then("it should report the number of loaded tools")]
fn then_report_tool_count(world: &mut QuectoWorld) {
    let info = world.agent_info.as_ref().expect("no agent info");
    assert!(
        info.tool_count > 0,
        "expected tool_count > 0, got {}",
        info.tool_count
    );
}

// ===========================================================================
