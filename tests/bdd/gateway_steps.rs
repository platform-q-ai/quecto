use super::*;

// Gateway Steps (shared by cron and heartbeat gateway scenarios)
// ===========================================================================

/// Set up a mock gateway context: creates a recording mock agent and
/// an in-memory cron store, along with a workspace temp dir.
#[given("a running gateway with a mock LLM provider")]
fn given_running_gateway_with_mock_llm(world: &mut QuectoWorld) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    world.mock_agent_messages = messages.clone();

    // Store the response as a simple default; scenarios that need
    // a specific response override it via "the mock LLM returns..." step.
    let agent = RecordingMockAgent {
        response: "OK".to_string(),
        messages,
    };
    // Store the agent as an Arc<dyn AgentLoop> on the world via a dedicated field.
    // We keep the concrete agent in _gateway_mock_agent.
    world._gateway_mock_agent = Some(DebugAgent(Arc::new(agent)));

    // In-memory cron store for cron scenarios.
    let store = Arc::new(InMemoryCronStore::new());
    world.gateway_cron_store = Some(store);

    // Workspace temp dir for heartbeat scenarios.
    let td = TempDir::new().expect("temp dir");
    world.heartbeat_workspace = Some(td.path().to_path_buf());
    world._extra_temp_dirs.push(td);

    // Default config.
    world.gateway_tick_config = Some(Config::default());
}

/// Override the gateway mock agent response.
#[given(expr = "the gateway agent responds with {string}")]
fn given_gateway_agent_responds_with(world: &mut QuectoWorld, response: String) {
    let messages = world.mock_agent_messages.clone();
    let agent = RecordingMockAgent { response, messages };
    world._gateway_mock_agent = Some(DebugAgent(Arc::new(agent)));
}

#[then(expr = "the mock LLM should receive a request containing {string}")]
fn then_mock_llm_received_containing(world: &mut QuectoWorld, expected: String) {
    let msgs = world.mock_agent_messages.lock().unwrap();
    assert!(
        msgs.iter().any(|m| m.contains(&expected)),
        "expected a request containing '{}', got: {:?}",
        expected,
        *msgs
    );
}

#[then("the mock LLM should not receive any requests")]
fn then_mock_llm_no_requests(world: &mut QuectoWorld) {
    let msgs = world.mock_agent_messages.lock().unwrap();
    assert!(
        msgs.is_empty(),
        "expected no LLM requests, got: {:?}",
        *msgs
    );
}

// ===========================================================================
