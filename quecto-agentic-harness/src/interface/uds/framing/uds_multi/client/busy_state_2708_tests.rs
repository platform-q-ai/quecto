//! #2708 live trial: a guarded harness answers its own `get_state` while a
//! turn is busy, from the refreshed snapshot, instead of queueing it behind
//! the turn (the Pocket relay reads state under a 15 s timeout). An
//! unguarded harness still queues it. Driven through the real multi-client
//! loop and socket.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use super::super::{MultiClientArgs, multi_client_loop};
use crate::application::agent_turn::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::conversation::value_objects::message::LlmResponse;
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::domain::shared::error::DomainError;

/// How long any one step may take before the test calls it a stall.
const STEP: Duration = Duration::from_secs(10);
/// How soon a busy `get_state` must be answered: far below the held turn.
const PROMPTLY: Duration = Duration::from_secs(3);
const KEY: &str = "cli:busy";

/// A streaming provider whose `held` replies (1-based) each stream one
/// delta, say they are in flight and hold until released; every other
/// reply answers at once.
#[derive(Debug, Default)]
struct HeldTurn {
    held: Vec<usize>,
    requests: Mutex<usize>,
    in_flight: Arc<Notify>,
    release: Arc<Notify>,
}

fn reply(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_owned()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

impl LlmProvider for HeldTurn {
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
        Box::pin(async { Ok(reply("unused")) })
    }
    fn chat_stream_incremental<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        let number = {
            let mut requests = self.requests.lock().unwrap();
            *requests += 1;
            *requests
        };
        let (in_flight, release) = (self.in_flight.clone(), self.release.clone());
        let hold = self.held.contains(&number);
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            tokio::spawn(async move {
                if hold {
                    let _ = tx.send(StreamEvent::TextDelta("work".into())).await;
                    in_flight.notify_one();
                    release.notified().await;
                }
                let _ = tx.send(StreamEvent::Done(reply("Answer."))).await;
            });
            rx
        })
    }
}

fn streaming_agent(provider: Arc<HeldTurn>) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 32,
        temperature: 0.0,
        retention: None,
        session_key: KEY.into(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    })
}

fn loop_args(base: &std::path::Path, agent: AgentLoopImpl, guarded: bool) -> MultiClientArgs<'_> {
    let parent_outcome =
        guarded.then(|| crate::composition::bootstrap::agent_turn::build_parent_outcome() as _);
    MultiClientArgs {
        parent_outcome,
        extensions: Default::default(),
        agent,
        workspace: base,
        messages: Vec::new(),
        model: "stub".into(),
        admission_slots: Vec::new(),
        session_key: KEY.into(),
        system_prompt: "system".into(),
        ext_registry: None,
        lifetime:
            crate::domain::agents::value_objects::harness_lifetime::HarnessLifetime::UntilLastClientDisconnects,
        notification_rx: None,
        subagent_registry: None,
        harness_lifecycle: None,
        workflow_state: None,
        workflow_config: None,
        broadcast_tx: None,
        parent_control: None,
        teardown_graph: None,
        turn_session: crate::composition::bootstrap::agent_turn::build_turn_session,
        environment_control: None,
    }
}

type Lines = tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>>;

/// Read events until one matches `wanted` within `bound`, returning every
/// event read on the way (the match last); `None` on the bound.
async fn until(
    lines: &mut Lines,
    bound: Duration,
    wanted: impl Fn(&serde_json::Value) -> bool,
) -> Option<Vec<serde_json::Value>> {
    let mut read = Vec::new();
    let found = tokio::time::timeout(bound, async {
        loop {
            let line = lines.next_line().await.unwrap().expect("socket open");
            let event: serde_json::Value = serde_json::from_str(&line).unwrap();
            let done = wanted(&event);
            read.push(event);
            if done {
                return;
            }
        }
    })
    .await;
    found.ok().map(|()| read)
}

async fn send(
    writer: &mut tokio::io::WriteHalf<tokio::net::UnixStream>,
    command: serde_json::Value,
) {
    let line = format!("{command}\n");
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
}

fn response_to(id: &'static str) -> impl Fn(&serde_json::Value) -> bool {
    move |event| event["type"] == "response" && event["id"] == id
}

struct Harness {
    provider: Arc<HeldTurn>,
    lines: Lines,
    writer: tokio::io::WriteHalf<tokio::net::UnixStream>,
    thread: std::thread::JoinHandle<i32>,
}

/// Start a loop (guarded or not) whose `held` replies hold, connect, and
/// hold a prompt's turn in flight.
async fn holding(dir: &std::path::Path, guarded: bool, held: Vec<usize>) -> Harness {
    let socket_path = dir.join("busy.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Arc::new(HeldTurn {
        held,
        ..HeldTurn::default()
    });
    let (agent_provider, base) = (provider.clone(), dir.to_path_buf());
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::UnixListener::from_std(listener).unwrap();
            let store = crate::composition::sessions::build_session_handles(
                crate::interface::uds::controllers::uds::dispatch_session_roster_tests::loop_inputs(
                    &base, KEY,
                ),
            );
            let catalogue = crate::composition::catalogue::build_catalogue_handles(&base, None);
            let args = loop_args(&base, streaming_agent(agent_provider), guarded);
            multi_client_loop(args, listener, &store, &catalogue).await
        })
    });
    let client = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let (reader, mut writer) = tokio::io::split(client);
    let lines = tokio::io::BufReader::new(reader).lines();
    let held = provider.in_flight.notified();
    send(
        &mut writer,
        serde_json::json!({"type": "prompt", "id": "go", "message": "Work"}),
    )
    .await;
    tokio::time::timeout(STEP, held)
        .await
        .expect("the turn is in flight");
    Harness {
        provider,
        lines,
        writer,
        thread,
    }
}

async fn holding_a_turn(dir: &std::path::Path, guarded: bool) -> Harness {
    holding(dir, guarded, vec![1]).await
}

impl Harness {
    /// Release the held reply, then wait until the next held one is in
    /// flight.
    async fn release_until_the_next_is_held(&mut self) {
        let next = self.provider.in_flight.notified();
        self.provider.release.notify_one();
        tokio::time::timeout(STEP, next)
            .await
            .expect("the next turn is in flight");
    }

    /// Ask `get_state` as `id`; the answer must come before the held turn
    /// ends.
    async fn state_now(&mut self, id: &'static str) -> serde_json::Value {
        send(
            &mut self.writer,
            serde_json::json!({"type": "get_state", "id": id}),
        )
        .await;
        let read = until(&mut self.lines, PROMPTLY, response_to(id))
            .await
            .expect("a busy get_state is answered while the turn holds");
        let answer = read.last().unwrap().clone();
        assert_eq!(answer["success"], true, "{answer}");
        answer["data"].clone()
    }

    async fn finish(self) {
        let Harness {
            lines,
            writer,
            thread,
            ..
        } = self;
        drop(writer);
        drop(lines);
        let joined = tokio::task::spawn_blocking(move || thread.join());
        let code = tokio::time::timeout(STEP, joined)
            .await
            .expect("the loop ends once the last client leaves")
            .expect("the join task ran")
            .expect("the loop did not panic");
        assert_eq!(code, 0);
    }
}

#[tokio::test]
async fn issue_2708_busy_state_a_guarded_get_state_is_answered_mid_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = holding_a_turn(dir.path(), true).await;

    send(
        &mut harness.writer,
        serde_json::json!({"type": "get_state", "id": "mid"}),
    )
    .await;
    let read = until(&mut harness.lines, PROMPTLY, response_to("mid"))
        .await
        .expect("a busy get_state is answered while the turn holds");
    assert!(
        read.iter().all(|event| event["type"] != "agent_end"),
        "answered before the turn ended: {read:?}"
    );
    let state = &read.last().unwrap()["data"];
    assert_eq!(read.last().unwrap()["success"], true, "{state}");
    assert_eq!(state["state"], "thinking", "{state}");
    assert_eq!(state["sessionKey"], KEY, "{state}");
    assert_eq!(state["turnOutcomeVersion"], 1, "{state}");
    assert!(state["outcomeEpoch"].is_string(), "{state}");
    assert!(state["latestTurnOutcome"].is_null(), "{state}");
    let generation = state["generation"].as_u64().expect("a generation");

    // A current cursor gets the unchanged marker, with the session key and
    // the outcome members.
    let since = serde_json::json!({"type": "get_state", "id": "same", "since": generation});
    send(&mut harness.writer, since).await;
    let read = until(&mut harness.lines, PROMPTLY, response_to("same"))
        .await
        .expect("a busy get_state since the cursor is answered too");
    let unchanged = &read.last().unwrap()["data"];
    assert_eq!(unchanged["unchanged"], true, "{unchanged}");
    assert_eq!(unchanged["generation"], generation, "{unchanged}");
    assert_eq!(unchanged["sessionKey"], KEY, "{unchanged}");
    assert_eq!(
        unchanged["outcomeEpoch"], state["outcomeEpoch"],
        "{unchanged}"
    );
    assert!(unchanged["latestTurnOutcome"].is_null(), "{unchanged}");

    harness.provider.release.notify_one();
    let read = until(&mut harness.lines, STEP, |event| {
        event["type"] == "turn_outcome"
    })
    .await
    .expect("the turn settles");
    assert_eq!(read.last().unwrap()["outcome"], "stalled");
    harness.finish().await;
}

#[tokio::test]
async fn issue_2708_busy_state_an_unguarded_get_state_still_waits_for_the_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = holding_a_turn(dir.path(), false).await;

    send(
        &mut harness.writer,
        serde_json::json!({"type": "get_state", "id": "mid"}),
    )
    .await;
    let early = until(
        &mut harness.lines,
        Duration::from_millis(500),
        response_to("mid"),
    )
    .await;
    assert!(early.is_none(), "legacy: queued behind the turn: {early:?}");

    harness.provider.release.notify_one();
    let read = until(&mut harness.lines, STEP, response_to("mid"))
        .await
        .expect("answered once the turn ended");
    assert!(
        read.iter().any(|event| event["type"] == "agent_end"),
        "{read:?}"
    );
    assert!(
        read.last().unwrap()["data"]
            .get("turnOutcomeVersion")
            .is_none()
    );
    harness.finish().await;
}

/// The relay's gap: the correction turn starts right after `agent_end`, so
/// a `get_state` sent then is answered from the snapshot too.
#[tokio::test]
async fn issue_2708_busy_state_the_correction_turn_is_answered_mid_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = holding(dir.path(), true, vec![1, 2]).await;
    harness.release_until_the_next_is_held().await;
    let state = harness.state_now("correcting").await;
    assert_eq!(state["state"], "thinking", "{state}");
    assert_eq!(state["sessionKey"], KEY, "{state}");
    assert!(
        state["latestTurnOutcome"].is_null(),
        "nothing settled yet: {state}"
    );
    harness.provider.release.notify_one();
    let read = until(&mut harness.lines, STEP, |event| {
        event["type"] == "turn_outcome"
    })
    .await
    .expect("the correction settles");
    assert_eq!(read.last().unwrap()["outcome"], "stalled");
    harness.finish().await;
}

/// The relay's other gap: an instruction queued behind the turn runs right
/// after its `turn_outcome` (here the prompt is corrected, request 2, and
/// stalls; the follow-up is request 3); mid-way the answer carries that
/// outcome.
#[tokio::test]
async fn issue_2708_busy_state_a_queued_follow_up_is_answered_with_the_settled_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = holding(dir.path(), true, vec![1, 3]).await;
    let queued = serde_json::json!({
        "type": "prompt", "id": "next", "message": "More", "streamingBehavior": "followUp",
    });
    send(&mut harness.writer, queued).await;
    // The reader has the follow-up well before the held reply resumes.
    tokio::time::sleep(Duration::from_millis(200)).await;
    harness.release_until_the_next_is_held().await;
    let state = harness.state_now("draining").await;
    assert_eq!(state["state"], "thinking", "{state}");
    assert_eq!(state["latestTurnOutcome"]["outcome"], "stalled", "{state}");
    let generation = state["generation"].as_u64().expect("a generation");
    let since = serde_json::json!({"type": "get_state", "id": "same", "since": generation});
    send(&mut harness.writer, since).await;
    let read = until(&mut harness.lines, PROMPTLY, response_to("same"))
        .await
        .expect("a busy get_state since the cursor is answered too");
    let unchanged = &read.last().unwrap()["data"];
    assert_eq!(unchanged["unchanged"], true, "{unchanged}");
    assert_eq!(
        unchanged["latestTurnOutcome"], state["latestTurnOutcome"],
        "{unchanged}"
    );
    harness.provider.release.notify_one();
    until(&mut harness.lines, STEP, |event| {
        event["type"] == "turn_outcome" && event["outcome"] == "stalled"
    })
    .await
    .expect("the drained turn settles");
    harness.finish().await;
}
