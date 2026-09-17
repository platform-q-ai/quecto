//! Retained context over the real UDS agent loop (D9 #1978): a real
//! `run_uds_loop` over the composed file retention store
//! (`composition::sessions::build_retention_handles`), a scripted provider
//! that drives an oversized stub tool past the collapse threshold and then
//! recalls through the real `recall` tool; every assertion reads the
//! loop's own wire events and the on-disk `spill.jsonl`.
use super::*;
use quecto::application::agent_loop::AgentLoopConfig;
use quecto::interface::cli::uds::{UdsLoopArgs, run_uds_loop};
use std::io::{Read, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

const STUB_BYTES: usize = 2400;
const PROMPT: &str = "exercise retained context";

fn stub_content(n: u64) -> String {
    format!("STUB{n} ").repeat(STUB_BYTES / 6)
}

/// The provider script of the scenario: two stub results (the first
/// collapses once the second lands, `context_collapse_after_tool_calls:
/// 1`), then `recall("list")`, the known id, the unknown id, then text.
#[derive(Debug)]
struct RetainedContextProvider {
    script: Mutex<std::collections::VecDeque<LlmResponse>>,
}

fn call(id: &str, name: &str, args: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args.into(),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

impl LlmProvider for RetainedContextProvider {
    fn name(&self) -> &str {
        "retained-context-script"
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let next = self.script.lock().unwrap().pop_front();
        Box::pin(async move {
            Ok(next.unwrap_or(LlmResponse {
                content: Some("retained context exercised".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            }))
        })
    }
}

/// A tool whose result is far larger than a collapse stub.
struct OversizedStubTool;

impl Tool for OversizedStubTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "stub".into(),
            description: "oversized stub".into(),
            parameters_schema: r#"{"type":"object","properties":{"n":{"type":"integer"}}}"#.into(),
        }
    }
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let n = serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|v| v.get("n").and_then(|n| n.as_u64()))
            .unwrap_or(0);
        Box::pin(async move {
            Ok(ToolResult {
                content: stub_content(n),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

#[given(
    expr = "a real UDS agent for session {string} over the file retention store that collapses after one tool result"
)]
fn given_real_retention_agent(world: &mut QuectoWorld, session_name: String) {
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");
    let retention = quecto::composition::sessions::build_retention_handles(&base);
    let session_key = Session::build_key("cli", &session_name);
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(OversizedStubTool));
    registry.register(Arc::new(RecallTool::new(
        retention.recall.clone(),
        session_key.clone(),
    )));
    let provider: Arc<dyn LlmProvider> = Arc::new(RetainedContextProvider {
        script: Mutex::new(
            vec![
                call("c1", "stub", r#"{"n":1}"#),
                call("c2", "stub", r#"{"n":2}"#),
                call("c3", "recall", r#"{"id":"list"}"#),
                call("c4", "recall", r#"{"id":"turn1:stub:0"}"#),
                call("c5", "recall", r#"{"id":"turn9:stub:0"}"#),
            ]
            .into(),
        ),
    });
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "retained-context-model".into(),
        max_tokens: 512,
        temperature: 0.0,
        retention: Some(retention.context.clone()),
        session_key: session_key.clone(),
        context_collapse_after_tool_calls: 1,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 0,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    world.retained_context_run = Some(RetainedContextRun {
        agent: Some(agent),
        retention: Some(retention),
        session_key,
        events: Vec::new(),
    });
}

#[when(
    "the model runs the stub tool twice, then recalls the index, the collapsed id and an unknown id"
)]
fn when_model_recalls(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.clone().expect("base dir");
    let run = world
        .retained_context_run
        .as_mut()
        .expect("agent not built");
    let agent = run.agent.take().expect("agent");
    let retention = run.retention.take().expect("retention");
    let session_key = run.session_key.clone();
    let (server, client) = std::os::unix::net::UnixStream::pair().expect("socket pair");
    let commands = [
        serde_json::json!({"type":"prompt","id":"p1","message":PROMPT}).to_string(),
        serde_json::json!({"type":"get_messages","id":"m1","count":100}).to_string(),
    ];
    let stdin: Vec<u8> = commands
        .iter()
        .flat_map(|l| format!("{l}\n").into_bytes())
        .collect();
    let socket_path = base.join("retained-context.sock");
    let handle = std::thread::spawn(move || {
        run_uds_loop(UdsLoopArgs {
            agent,
            retention: Some(retention),
            base_dir: &base,
            workspace: &base,
            identity: quecto::domain::session_identity::SessionIdentity::from_persisted_key(
                &session_key,
            ),
            model: "retained-context-model".into(),
            ephemeral: false,
            system_prompt: String::new(),
            socket_path,
            socket_override: Some(server),
            sessions: quecto::composition::sessions::build_session_handles,
            catalogue: quecto::composition::catalogue::build_catalogue_handles(&base, None),
            ext_registry: None,
            lifetime: quecto::domain::harness_lifetime::HarnessLifetime::UntilLastClientDisconnects,
            notification_rx: None,
            subagent_registry: None,
            harness_lifecycle: None,
            workflow_state: None,
            workflow_config: None,
            broadcast_tx: None,
            parent_control: None,
            teardown_graph: None,
        })
    });
    let mut client = client;
    client.write_all(&stdin).expect("write commands");
    client
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown write");
    let mut bytes = Vec::new();
    client.read_to_end(&mut bytes).expect("read events");
    let exit = handle.join().expect("loop thread");
    assert_eq!(exit, 0, "the loop exits cleanly");
    run.events = String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
}

fn tool_result_text(world: &QuectoWorld, tool_call_id: &str) -> (String, bool) {
    let run = world.retained_context_run.as_ref().expect("run");
    let event = run
        .events
        .iter()
        .find(|e| e["type"] == "tool_execution_end" && e["toolCallId"] == tool_call_id)
        .unwrap_or_else(|| panic!("no tool_execution_end for {tool_call_id}"));
    let text = event["result"]["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string();
    (text, event["isError"].as_bool().unwrap_or(false))
}

#[then("the recall index answered exactly the user prompt and both stub results in append order")]
fn then_index_exact(world: &mut QuectoWorld) {
    let (text, is_error) = tool_result_text(world, "c3");
    let prompt_tokens = Message::estimate_tokens(PROMPT);
    let stub_tokens = Message::estimate_tokens(&stub_content(1));
    let expected = format!(
        "Spilled outputs (3 entries):\n  turn0:msg:user — {PROMPT} ({prompt_tokens} tokens)\n  \
         turn1:stub:0 — {{\"n\":1}} ({stub_tokens} tokens)\n  turn2:stub:0 — {{\"n\":2}} ({stub_tokens} tokens)\n"
    );
    assert!(!is_error);
    assert_eq!(text, expected);
}

#[then("the recall of the collapsed id answered the full stub output")]
fn then_known_recall(world: &mut QuectoWorld) {
    let (text, is_error) = tool_result_text(world, "c4");
    assert!(!is_error);
    assert_eq!(text, stub_content(1));
}

#[then(expr = "the recall of the unknown id answered exactly {string} as an error")]
fn then_unknown_recall(world: &mut QuectoWorld, expected: String) {
    let (text, is_error) = tool_result_text(world, "c5");
    assert!(is_error, "an unknown id is a tool error");
    assert_eq!(text, expected);
}

#[then(expr = "the first stub result is shown collapsed to the recall stub for {string}")]
fn then_collapsed_stub(world: &mut QuectoWorld, spill_id: String) {
    let run = world.retained_context_run.as_ref().expect("run");
    let response = run
        .events
        .iter()
        .find(|e| e["type"] == "response" && e["id"] == "m1")
        .expect("get_messages response");
    let messages = response["data"]["messages"].as_array().expect("messages");
    let stub_tokens = Message::estimate_tokens(&stub_content(1));
    let expected = quecto::application::context_pruning::collapse_stub(
        "stub",
        r#"{"n":1}"#,
        stub_tokens,
        &spill_id,
    );
    assert!(
        messages
            .iter()
            .any(|m| m["collapsed"] == true && m["content"] == expected),
        "no collapsed stub {expected:?} in {messages:#?}"
    );
    // `context_collapse_after_tool_calls: 1`: only the most recent tool
    // result stays in full — the unknown-id refusal of the last recall.
    let full: Vec<&serde_json::Value> = messages
        .iter()
        .filter(|m| m["role"] == "tool" && m["collapsed"] == false)
        .collect();
    assert_eq!(full.len(), 1, "one live tool result: {full:#?}");
    assert_eq!(
        full[0]["content"],
        "No spilled output found for id: turn9:stub:0"
    );
}

#[then(expr = "the file retention store of session {string} holds {string} and {string} on disk")]
fn then_spill_file(world: &mut QuectoWorld, session_name: String, first: String, second: String) {
    let base = world.cli_context.base_dir.clone().expect("base dir");
    let file = base.join(format!("sessions/cli_{session_name}/spill.jsonl"));
    let content =
        std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
    let ids: Vec<String> = content
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("spill record")["id"]
                .as_str()
                .expect("id")
                .to_string()
        })
        .collect();
    assert!(ids.contains(&first), "{ids:?}");
    assert!(ids.contains(&second), "{ids:?}");
    assert_eq!(ids[0], "turn0:msg:user", "oldest first: {ids:?}");
}

/// One real-loop run of the retained-context scenario.
pub struct RetainedContextRun {
    agent: Option<AgentLoopImpl>,
    retention: Option<quecto::interface::cli::retention_handles::RetentionHandles>,
    session_key: String,
    events: Vec<serde_json::Value>,
}

impl std::fmt::Debug for RetainedContextRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedContextRun")
            .field("session_key", &self.session_key)
            .field("events", &self.events.len())
            .finish()
    }
}
