use super::*;
use quecto::domain::agent::AgentProgressEvent;
use quecto::domain::message::Message;

fn started(id: &str, name: &str) -> AgentProgressEvent {
    AgentProgressEvent::ToolStarted {
        tool_call_id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
    }
}
fn finished(id: &str, name: &str, is_error: bool) -> AgentProgressEvent {
    AgentProgressEvent::ToolFinished {
        tool_call_id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
        result_content: String::new(),
        duration_ms: 1,
        is_error,
    }
}

#[given("a child agent is processing a turn with tool activity")]
fn child_processing_tool(world: &mut QuectoWorld) {
    world.live_execution_state = Some(cli::live_execution_state_for_events(&[started(
        "call-1", "bash",
    )]));
}

#[when("the parent requests the child's state while a tool is running")]
fn request_running_state(_world: &mut QuectoWorld) {}

#[then("the state should identify the current execution phase and tool")]
fn state_identifies_tool(world: &mut QuectoWorld) {
    let state = world
        .live_execution_state
        .as_ref()
        .expect("execution state");
    assert_eq!(state["execution"]["phase"], "runningTool");
    assert_eq!(state["execution"]["currentTool"]["name"], "bash");
    assert_eq!(state["execution"]["currentTool"]["callId"], "call-1");
}

#[given("a child agent has completed tools during the current activity window")]
fn child_completed_tools(world: &mut QuectoWorld) {
    world.live_execution_state = Some(cli::live_execution_state_for_events(&[
        started("call-1", "read"),
        finished("call-1", "read", false),
        started("call-2", "bash"),
        finished("call-2", "bash", true),
    ]));
}

#[when("the parent requests the child's state")]
fn request_state(_world: &mut QuectoWorld) {}

#[then("the state should summarize recent completed and failed tool calls")]
fn state_summarizes_tools(world: &mut QuectoWorld) {
    let state = world
        .live_execution_state
        .as_ref()
        .expect("execution state");
    assert_eq!(state["execution"]["progress"]["state"], "advancing");
    assert_eq!(state["execution"]["progress"]["windowSeconds"], 120);
    assert_eq!(state["execution"]["progress"]["toolCallsCompleted"], 2);
    assert_eq!(state["execution"]["progress"]["toolCallsFailed"], 1);
}

#[given("a child agent appends conversation messages during an active turn")]
fn child_appends_messages(world: &mut QuectoWorld) {
    let messages: Arc<[Message]> =
        vec![Message::user("work"), Message::assistant("done", vec![])].into();
    world.live_execution_state = Some(cli::live_execution_state_for_events(&[
        AgentProgressEvent::ConversationChanged { messages },
    ]));
}

#[when("the parent requests the child's state before the turn completes")]
fn request_inflight_state(_world: &mut QuectoWorld) {}

#[then("the state message count should include the in-flight committed messages")]
fn state_counts_inflight_messages(world: &mut QuectoWorld) {
    let state = world.live_execution_state.as_ref().unwrap();
    assert_eq!(state["messageCount"], 2);
    assert!(state["execution"]["activityGeneration"].as_u64().unwrap() >= 2);
}

#[given("a child agent has completed its active turn")]
fn child_completed_turn(world: &mut QuectoWorld) {
    world.live_execution_state = Some(cli::completed_live_execution_state(&[
        started("call-1", "read"),
        finished("call-1", "read", false),
    ]));
}

#[then("the state should report idle execution without a current tool")]
fn state_is_idle(world: &mut QuectoWorld) {
    let state = world.live_execution_state.as_ref().unwrap();
    assert_eq!(state["execution"]["phase"], "idle");
    assert!(state["execution"]["currentTool"].is_null());
}

#[given("the agent command tool is available")]
fn command_tool_available(world: &mut QuectoWorld) {
    let registry = Arc::new(Mutex::new(Default::default()));
    world.agent_cmd_tool = Some(quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new(
        registry,
    ));
}

#[when("a parent inspects its command guidance")]
fn inspect_command_guidance(_world: &mut QuectoWorld) {}

#[then("the guidance should distinguish live state from committed transcript history")]
fn guidance_distinguishes_contracts(world: &mut QuectoWorld) {
    let description = world
        .agent_cmd_tool
        .as_ref()
        .unwrap()
        .definition()
        .description;
    assert!(description.contains("get_state only for occasional live supervision"));
    assert!(description.contains("get_messages"));
    assert!(description.contains("default unread report"));
    assert!(description.contains("no count/before"));
}
