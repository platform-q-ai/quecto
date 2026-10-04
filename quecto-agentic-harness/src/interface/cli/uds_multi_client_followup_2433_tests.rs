//! #2433: a `prompt` with `streamingBehavior: "followUp"` that arrives while
//! the model's reply is still streaming — sent by the very connection whose
//! extension tool just returned — is queued for after the run and leaves the
//! in-flight reply alone: the reply streams on to its end, and the
//! follow-up runs next. Driven through the real multi-client loop and socket.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use super::super::{MultiClientArgs, multi_client_loop};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::provider::StreamEvent;

/// The follow-up the extension sends mid-reply.
const FOLLOW_UP: &str = "the background job finished";
/// Deltas the held reply streams once released: far more than every
/// channel between the provider and the agent loop holds, so a consumer
/// that stopped reading would hold the reply.
const RELEASED_DELTAS: usize = 300;
/// How long any one step may take before the test calls it a stall.
const STEP: Duration = Duration::from_secs(10);

/// A streaming provider: the first reply calls the extension tool; the
/// second streams one delta, says it is in flight and holds until released;
/// every later reply answers at once. Each request's messages are kept.
#[derive(Debug)]
struct HeldReply {
    requests: Mutex<Vec<Vec<Message>>>,
    in_flight: Arc<Notify>,
    release: Arc<Notify>,
}

fn reply(content: &str, tool_calls: Vec<ToolCall>) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_owned()),
        tool_calls,
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

impl LlmProvider for HeldReply {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "held"
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async { Ok(reply("unused", vec![])) })
    }
    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        let number = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.messages.to_vec());
            requests.len()
        };
        let (in_flight, release) = (self.in_flight.clone(), self.release.clone());
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            tokio::spawn(async move {
                let done = match number {
                    1 => {
                        let call = ToolCall {
                            id: "call_job".into(),
                            name: "start_job".into(),
                            arguments: "{}".into(),
                        };
                        reply("", vec![call])
                    }
                    2 => {
                        let _ = tx.send(StreamEvent::TextDelta("wait".into())).await;
                        in_flight.notify_one();
                        release.notified().await;
                        for _ in 0..RELEASED_DELTAS {
                            let _ = tx.send(StreamEvent::TextDelta(".".into())).await;
                        }
                        reply("waiting", vec![])
                    }
                    _ => reply("resumed", vec![]),
                };
                let _ = tx.send(StreamEvent::Done(done)).await;
            });
            rx
        })
    }
}

fn streaming_agent(provider: Arc<HeldReply>) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 32,
        temperature: 0.0,
        retention: None,
        session_key: "cli:followup".into(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
}

fn loop_args(base: &std::path::Path, agent: AgentLoopImpl) -> MultiClientArgs<'_> {
    MultiClientArgs {
        extensions: Default::default(),
        agent,
        workspace: base,
        messages: Vec::new(),
        model: "stub".into(),
        admission_slots: Vec::new(),
        session_key: "cli:followup".into(),
        system_prompt: "system".into(),
        ext_registry: None,
        lifetime: crate::domain::harness_lifetime::HarnessLifetime::UntilLastClientDisconnects,
        notification_rx: None,
        subagent_registry: None,
        harness_lifecycle: None,
        workflow_state: None,
        workflow_config: None,
        broadcast_tx: None,
        parent_control: None,
        teardown_graph: None,
        environment_control: None,
    }
}

type Lines = tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>>;

/// Read events until one matches `wanted`, failing on a stall.
async fn until(
    lines: &mut Lines,
    wanted: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    tokio::time::timeout(STEP, async {
        loop {
            let line = lines.next_line().await.unwrap().expect("socket open");
            let event: serde_json::Value = serde_json::from_str(&line).unwrap();
            if wanted(&event) {
                return event;
            }
        }
    })
    .await
    .expect("the agent stalled")
}

async fn send(
    writer: &mut tokio::io::WriteHalf<tokio::net::UnixStream>,
    command: serde_json::Value,
) {
    writer
        .write_all(format!("{command}\n").as_bytes())
        .await
        .unwrap();
    writer.flush().await.unwrap();
}

fn response_to(id: &'static str) -> impl Fn(&serde_json::Value) -> bool {
    move |event| event["type"] == "response" && event["id"] == id
}

#[tokio::test]
async fn a_follow_up_sent_mid_reply_waits_for_the_run_and_leaves_the_reply_streaming() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("followup.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Arc::new(HeldReply {
        requests: Mutex::new(Vec::new()),
        in_flight: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
    });
    let agent_provider = provider.clone();
    let harness = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::UnixListener::from_std(listener).unwrap();
            let store = crate::composition::sessions::build_session_handles(
                crate::interface::cli::uds::dispatch_session_roster_tests::loop_inputs(
                    dir.path(),
                    "cli:followup",
                ),
            );
            multi_client_loop(
                loop_args(dir.path(), streaming_agent(agent_provider)),
                listener,
                &store,
                &crate::composition::catalogue::build_catalogue_handles(dir.path(), None),
            )
            .await
        })
    });

    // The extension: one connection registers the tool, prompts, answers
    // the tool call and then — the reply still streaming — sends the
    // follow-up, as the reported extension did.
    let client = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let (reader, mut writer) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(reader).lines();
    let tool = serde_json::json!({
        "name": "start_job",
        "description": "Start a background job",
        "parametersSchema": "{\"type\":\"object\"}",
    });
    send(
        &mut writer,
        serde_json::json!({"type": "register_tools", "id": "reg", "tools": [tool]}),
    )
    .await;
    let registered = until(&mut lines, response_to("reg")).await;
    assert_eq!(registered["success"], true, "{registered}");

    send(
        &mut writer,
        serde_json::json!({"type": "prompt", "id": "go", "message": "start it"}),
    )
    .await;
    let call = until(&mut lines, |e| e["type"] == "execute_tool").await;
    let started = serde_json::json!({
        "type": "tool_result",
        "toolCallId": call["toolCallId"],
        "content": "started; end your turn",
    });
    send(&mut writer, started).await;
    tokio::time::timeout(STEP, provider.in_flight.notified())
        .await
        .expect("the reply after the tool result is in flight");

    let follow_up = serde_json::json!({
        "type": "prompt",
        "id": "push",
        "message": FOLLOW_UP,
        "streamingBehavior": "followUp",
    });
    send(&mut writer, follow_up).await;
    // The reader has the follow-up well before the reply resumes.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        provider.requests.lock().unwrap().len(),
        2,
        "nothing ran yet"
    );
    provider.release.notify_one();

    let first_run = until(&mut lines, response_to("go")).await;
    assert_eq!(
        first_run["success"], true,
        "the held reply completed: {first_run}"
    );
    let follow_up_run = until(&mut lines, response_to("push")).await;
    assert_eq!(follow_up_run["success"], true, "{follow_up_run}");

    let requests = provider.requests.lock().unwrap().clone();
    let carries_follow_up =
        |messages: &[Message]| messages.iter().any(|m| m.content.contains(FOLLOW_UP));
    assert_eq!(
        requests.len(),
        3,
        "tool call, held reply, then the follow-up"
    );
    assert!(
        !carries_follow_up(&requests[1]),
        "the held reply's request never saw the follow-up"
    );
    assert!(
        carries_follow_up(&requests[2]),
        "the follow-up ran after the run"
    );

    drop(writer);
    drop(lines);
    // Joined off the runtime, so a loop that never ends fails the test at
    // the bound rather than blocking it for ever.
    let joined = tokio::task::spawn_blocking(move || harness.join());
    let code = tokio::time::timeout(STEP, joined)
        .await
        .expect("the loop ends once the last client leaves")
        .expect("the join task ran")
        .expect("the loop did not panic");
    assert_eq!(code, 0);
}
