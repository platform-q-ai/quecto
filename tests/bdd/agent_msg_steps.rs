use super::*;

// Agent Tools (Message, Spawn) Steps
// ===========================================================================

#[given(expr = "a message tool with default target {string}")]
fn given_message_tool(world: &mut QuectoWorld, target: String) {
    let mut bus = MessageBus::new(16);
    let sender = bus.outbound_sender();
    let receiver = bus.take_outbound_receiver().unwrap();
    world.message_bus_receiver = Some(receiver);

    let tool = MessageTool::new(sender, Some(target));
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    world.tool_registry = Some(registry);
}

#[when(expr = "the agent sends a message {string} via the message tool")]
fn when_send_via_message_tool(world: &mut QuectoWorld, text: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let args = serde_json::json!({"text": text}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("message", &args))
        .unwrap();
    world.tool_result = Some(Ok(result));
}

#[then(expr = "the outbound bus should have a message for {string} with text {string}")]
fn then_outbound_bus_has_message(world: &mut QuectoWorld, target: String, text: String) {
    let receiver = world
        .message_bus_receiver
        .as_mut()
        .expect("no bus receiver");
    let msg = receiver.try_recv().expect("no message on outbound bus");
    assert_eq!(
        msg.target, target,
        "expected target '{}', got '{}'",
        target, msg.target
    );
    assert_eq!(
        msg.text, text,
        "expected text '{}', got '{}'",
        text, msg.text
    );
}

#[given(expr = "a spawn tool with allowed agents {string} and {string}")]
fn given_spawn_tool(world: &mut QuectoWorld, agent1: String, agent2: String) {
    world.spawn_tool = Some(SpawnTool::new(vec![agent1, agent2], true));
}

#[when(expr = "the agent executes the spawn tool with task {string}")]
fn when_execute_spawn_tool(world: &mut QuectoWorld, task: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn tool not set");
    let args = serde_json::json!({"task": task}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(&args))
        .unwrap();
    world.spawn_result = Some(result);
}

#[when(expr = "the agent executes the spawn tool with task {string} and agent_id {string}")]
fn when_execute_spawn_with_agent(world: &mut QuectoWorld, task: String, agent_id: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn tool not set");
    let args = serde_json::json!({"task": task, "agent_id": agent_id}).to_string();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(&args))
        .unwrap();
    world.spawn_result = Some(result);
}

#[then("the spawn result should confirm the subagent was spawned")]
fn then_spawn_result_ok(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        !result.is_error,
        "expected spawn success, got error: {}",
        result.content
    );
    assert!(
        result.content.contains("spawned"),
        "expected 'spawned' in content: {}",
        result.content
    );
}

#[then(expr = "the spawn result should be an error mentioning {string}")]
fn then_spawn_result_error(world: &mut QuectoWorld, expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(result.is_error, "expected spawn error");
    assert!(
        result.content.contains(&expected),
        "expected error to mention '{}', got: {}",
        expected,
        result.content
    );
}

// ===========================================================================
