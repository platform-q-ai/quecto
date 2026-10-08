//! Fake-provider end-to-end run through the real agent loop and swarm tool.
use quecto::application::agent_loop::AgentLoopImpl;
use quecto::application::agent_turn::ports::AgentLoop;
use quecto::domain::conversation::value_objects::message::{LlmResponse, Message, ToolCall};
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::{
    filesystem::WriteTool, registry::ToolRegistryImpl, swarm::SwarmTool, swarm_bridge::SwarmContext,
};
use std::sync::Arc;

/// One step of the fake coordinator: a tool call built from the answers
/// of the swarm ops made so far (a claim's token feeds its submit).
type Step = fn(&[serde_json::Value]) -> (&'static str, serde_json::Value);

fn swarm(request: serde_json::Value) -> (&'static str, serde_json::Value) {
    ("swarm", request)
}

/// The answer of the `index`th swarm op (0-based).
fn answer(answers: &[serde_json::Value], index: usize) -> &serde_json::Value {
    &answers[index]
}

/// The coordinator's work, one board method per `swarm` call (#2281): it
/// defines verification, decomposes, blocks on a question, resolves it,
/// and verifies both tasks before completing. `write` stands in for the
/// program's file writes.
const STEPS: &[Step] = &[
    |_| {
        swarm(
            serde_json::json!({"op":"amend","goal":"ship feature","constraints":[],
            "criteria":[{"id":"tests","kind":"command","description":"acceptance passes"},
                {"id":"review","kind":"review","description":"independent review"}],
            "reason":"define verification"}),
        )
    },
    |_| {
        swarm(
            serde_json::json!({"op":"task_create","request":"build","title":"implement",
            "acceptance":["acceptance passes"],"dependencies":[]}),
        )
    },
    |a| {
        swarm(
            serde_json::json!({"op":"task_create","request":"review-task","title":"review",
            "acceptance":["independent review"],"dependencies":[answer(a, 1)["id"]]}),
        )
    },
    |a| swarm(serde_json::json!({"op":"claim","task_id":answer(a, 1)["id"]})),
    |a| {
        swarm(
            serde_json::json!({"op":"block","task_id":answer(a, 1)["id"],
            "token":answer(a, 3)["token"],"reason":"need schema"}),
        )
    },
    |_| swarm(serde_json::json!({"op":"task","task_id":1})),
    |_| {
        swarm(
            serde_json::json!({"op":"send","request":"schema","recipient":"coordinator",
            "body":"schema approved"}),
        )
    },
    |_| swarm(serde_json::json!({"op":"inbox"})),
    |a| swarm(serde_json::json!({"op":"ack","message_id":answer(a, 7)[0]["id"]})),
    |_| {
        (
            "write",
            serde_json::json!({"path":"acceptance.log","content":"PASS revision abc"}),
        )
    },
    |a| {
        swarm(
            serde_json::json!({"op":"submit","task_id":1,"token":answer(a, 5)["token"],
            "evidence":[{"artifact":"acceptance.log","revision":"abc"}]}),
        )
    },
    |a| {
        swarm(serde_json::json!({"op":"verify_task","task_id":1,
            "token":answer(a, 5)["token"],"revision":"abc"}))
    },
    |_| swarm(serde_json::json!({"op":"claim","task_id":2})),
    |_| {
        (
            "write",
            serde_json::json!({"path":"review.md","content":"Reviewed revision abc: approved"}),
        )
    },
    |a| {
        swarm(
            serde_json::json!({"op":"submit","task_id":2,"token":answer(a, 12)["token"],
            "evidence":[{"artifact":"review.md","revision":"abc"}]}),
        )
    },
    |a| {
        swarm(serde_json::json!({"op":"verify_task","task_id":2,
            "token":answer(a, 12)["token"],"revision":"abc"}))
    },
    |_| {
        swarm(
            serde_json::json!({"op":"evidence","criterion":"tests","artifact":"acceptance.log",
            "revision":"abc","kind":"command","passed":true}),
        )
    },
    |_| {
        swarm(
            serde_json::json!({"op":"evidence","criterion":"review","artifact":"review.md",
            "revision":"abc","kind":"review","passed":true}),
        )
    },
    |_| swarm(serde_json::json!({"op":"complete","revision":"abc"})),
];

#[tokio::test]
async fn fake_provider_decomposes_resolves_blocker_and_verifies_swarm() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Arc::new(directory.path().to_path_buf());
    let context = SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
    };
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    quecto::infrastructure::tools::call_work::off_the_runtime(|| context.create_run(&serde_json::json!({"goal":"ship","constraints":[],"criteria":[{"id":"tests","kind":"command","description":"pass"}],"member_limit":1,"deadline":deadline}),
        &quecto::domain::swarm::ProcessIdentity { pid: std::process::id(), started: quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap() }, None)).unwrap();
    let tool = SwarmTool::new().with_context(Some(context));
    let provider = Arc::new(MockProvider::new(STEPS));
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    registry.register(Arc::new(WriteTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
    )));
    let mut agent = AgentLoopImpl::new(test_config(provider.clone(), Box::new(registry)));
    let result = agent
        .process(&mut vec![Message::user("Complete the bounded swarm")])
        .await
        .unwrap();
    assert!(result.response.contains("Verified completion"));
    let context = SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
    };
    let summary =
        quecto::infrastructure::tools::call_work::off_the_runtime(|| context.summary()).unwrap();
    assert_eq!(
        (summary["status"].as_str(), summary["outcome"].as_str()),
        (Some("paused"), Some("succeeded"))
    );
    assert_eq!(summary["counts"]["completed"], 2);
    assert!(
        quecto::infrastructure::tools::call_work::off_the_runtime(|| context.events(0, 100))
            .unwrap()["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "blocked")
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("acceptance.log")).unwrap(),
        "PASS revision abc"
    );
    // One request per call and the final answer; every call answered.
    assert_eq!(provider.request_count(), STEPS.len() + 1);
    assert_eq!(provider.refusals(), Vec::<String>::new());
}

/// #2279 review L3: a structured op whose argument text holds `NaN`,
/// `Infinity` or `-Infinity` (which Python's `json.loads` reads) never
/// reaches the swarm tool: the agent loop's `ToolCall::argument_shape`
/// finds no JSON object in it and the member reads the loop's
/// invalid-arguments answer, with nothing called on the board. A number
/// that overflows (`1e400`) or a lone surrogate escape is JSON, so it
/// reaches the tool, which refuses it before the board.
#[tokio::test]
async fn a_non_finite_argument_is_answered_by_the_loop_before_the_tool() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Arc::new(directory.path().to_path_buf());
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let context = SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
    };
    let log = Arc::new(BoardCalls::default());
    assert!(context.board.record_in(log.clone()));
    let tool = SwarmTool::new().with_context(Some(context));
    let texts = [
        r#"{"op": "claim", "task_id": NaN}"#,
        r#"{"op": "claim", "task_id": Infinity}"#,
        r#"{"op": "claim", "task_id": -Infinity}"#,
    ];
    let reaching = [
        r#"{"op": "claim", "task_id": 1e400}"#,
        r#"{"op": "send", "request": "r", "recipient": "w", "body": "\ud800"}"#,
    ];
    let mut calls = text_response("");
    calls.content = None;
    for (index, text) in texts.iter().chain(&reaching).enumerate() {
        calls.tool_calls.push(ToolCall {
            id: format!("swarm-{index}"),
            name: "swarm".into(),
            arguments: (*text).to_owned(),
        });
    }
    let provider = Arc::new(MockProvider::queued(vec![calls, text_response("done")]));
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    let mut agent = AgentLoopImpl::new(test_config(provider.clone(), Box::new(registry)));
    let mut messages = vec![Message::user("claim task 1")];
    agent.process(&mut messages).await.unwrap();
    let answers: Vec<&str> = messages
        .iter()
        .filter(|message| {
            message.role == quecto::domain::conversation::value_objects::message::Role::Tool
        })
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(answers.len(), texts.len() + reaching.len(), "{answers:?}");
    for (answer, text) in answers.iter().zip(texts) {
        assert!(
            answer.starts_with(
                "the arguments for tool 'swarm' were not a JSON object, so it was not run"
            ) && answer.ends_with(&format!("Received: {text}")),
            "{answer}"
        );
    }
    for answer in &answers[texts.len()..] {
        assert!(
            answer.starts_with(r#"tool error: swarm: "arguments: "#),
            "{answer}"
        );
    }
    let recorded: Vec<(String, quecto::domain::swarm::BoardOpOutcome)> = log
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|record| (record.op.clone(), record.outcome))
        .collect();
    let refused = quecto::domain::swarm::BoardOpOutcome::Refused {
        kind: quecto::domain::swarm::RefusalKind::Invalid,
        committed: false,
    };
    assert_eq!(
        recorded,
        [("claim".to_owned(), refused), ("send".to_owned(), refused)],
        "only the two texts that reached the tool are recorded, refused"
    );
    assert_eq!(provider.request_count(), 2);
}

/// A running swarm with `coordinator` as its only member, over a fresh
/// board that records every call in the returned log.
fn recorded_run(directory: &tempfile::TempDir) -> (SwarmContext, Arc<BoardCalls>) {
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let context = SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
    };
    let log = Arc::new(BoardCalls::default());
    assert!(context.board.record_in(log.clone()));
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    let process = quecto::domain::swarm::ProcessIdentity {
        pid: std::process::id(),
        started: quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id())
            .unwrap(),
    };
    quecto::infrastructure::tools::call_work::off_the_runtime(|| {
        context.create_run(
            &serde_json::json!({"goal":"ship","constraints":[],
                "criteria":[{"id":"tests","kind":"command","description":"pass"}],
                "member_limit":1,"deadline":deadline}),
            &process,
            None,
        )
    })
    .unwrap();
    (context, log)
}

/// The admission reads and request records the board logged, in order, as
/// `(op, decision)`.
fn admission_trail(log: &BoardCalls) -> Vec<(String, Option<String>)> {
    log.0
        .lock()
        .unwrap()
        .iter()
        .filter(|record| matches!(record.op.as_str(), "_request_admission" | "_record_request"))
        .map(|record| (record.op.clone(), record.decision.clone()))
        .collect()
}

/// How [`gate_trail`] sends its requests: streaming (as members do) or
/// not, and with a transient failure of the first request's first send.
#[derive(Clone, Copy, Debug)]
struct Sending {
    streaming: bool,
    first_send_fails: bool,
}

/// Runs a member's agent loop, wired by the CLI's own
/// `wire_agent`, over a real board: three requests asking for two tool
/// calls, then one, then none. Answers the board's admission and usage
/// trail. Without streaming, retries are `RetryingProvider`'s, as the CLI
/// composes it; with streaming, the loop re-initiates the stream itself.
async fn gate_trail(sending: Sending) -> Vec<(String, Option<String>)> {
    let directory = tempfile::tempdir().unwrap();
    let (context, log) = recorded_run(&directory);
    let tool = SwarmTool::new().with_context(Some(context.clone()));
    let call = |id: &str, op: &str| ToolCall {
        id: id.into(),
        name: "swarm".into(),
        arguments: serde_json::json!({ "op": op }).to_string(),
    };
    let mut two_calls = text_response("");
    two_calls.content = None;
    two_calls.tool_calls = vec![call("a", "summary"), call("b", "inbox")];
    let mut one_call = text_response("");
    one_call.content = None;
    one_call.tool_calls = vec![call("c", "tasks")];
    let provider = Arc::new(MockProvider::queued(vec![
        two_calls,
        one_call,
        text_response("done"),
    ]));
    if sending.first_send_fails {
        provider.fail_next("HTTP 503 Service Unavailable");
    }
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    let mut config = test_config(provider.clone(), Box::new(registry));
    config.streaming = sending.streaming;
    if !sending.streaming {
        config.provider = Arc::new(
            quecto::infrastructure::providers::retry::RetryingProvider::new(
                provider.clone(),
                quecto::infrastructure::providers::retry::RetryConfig::no_delay(3),
            ),
        );
    }
    let mut agent =
        quecto::interface::cli::wire_swarm_agent(AgentLoopImpl::new(config), Some(context));
    let result = agent
        .process(&mut vec![Message::user("read the run")])
        .await
        .unwrap();
    assert_eq!(result.response, "done", "{sending:?}");
    let sends = 3 + usize::from(sending.first_send_fails);
    assert_eq!(provider.request_count(), sends, "{sending:?}");
    admission_trail(&log)
}

/// The trail of three requests (two tool calls, one, none), each first
/// send's model gate followed by `retries` retry gates for the first.
fn expected_trail(retries: usize) -> Vec<(String, Option<String>)> {
    let admission = |decision: &str| ("_request_admission".to_owned(), Some(decision.to_owned()));
    let recorded = ("_record_request".to_owned(), Some("recorded".to_owned()));
    let mut trail = vec![admission("model_gate")];
    trail.extend(std::iter::repeat_n(admission("retry_gate"), retries));
    trail.extend([
        recorded.clone(),
        admission("tool_gate"),
        admission("tool_gate"),
        admission("model_gate"),
        recorded.clone(),
        admission("tool_gate"),
        admission("model_gate"),
        recorded,
    ]);
    trail
}

/// #2339: through the real agent loop, wired as the CLI wires a member,
/// each model request reads the board's admission once before its first
/// send (`model_gate`) and records its usage once after, so here, with no
/// redelivery, `_record_request` and the model gate are 1:1. The other
/// reads are the tool gate, one before each tool call, needed because the
/// reply that asked for the tool took its own time (and its usage may just
/// have spent the budget); each is recorded as `tool_gate`, so the log
/// tells them apart. The real runs' ~2 reads per request were this 1 + ~1
/// tool call. Streaming (as members run) or not, the trail is the same.
#[tokio::test]
async fn each_model_request_is_admitted_once_and_each_tool_call_once() {
    for streaming in [false, true] {
        let trail = gate_trail(Sending {
            streaming,
            first_send_fails: false,
        })
        .await;
        assert_eq!(trail, expected_trail(0), "streaming: {streaming}");
    }
}

/// #2339 review L2: a first send that fails transiently is sent again
/// after a `retry_gate` read, never a second `model_gate`: by the stream's
/// re-initiation when streaming, by `RetryingProvider` when not.
#[tokio::test]
async fn a_resent_request_reads_the_retry_gate_not_a_second_model_gate() {
    for streaming in [false, true] {
        let trail = gate_trail(Sending {
            streaming,
            first_send_fails: true,
        })
        .await;
        assert_eq!(trail, expected_trail(1), "streaming: {streaming}");
    }
}

/// #2339: a member's admission records which gate read it: the first send
/// of a model request, a reattempt of it, or a tool call.
#[tokio::test]
async fn each_admission_gate_is_recorded_as_its_own_decision() {
    use quecto::application::providers::ports::RequestAdmission;
    use quecto::application::tools::ports::ToolExecutionAdmission;
    use quecto::domain::inference::value_objects::provider::RequestAttempt;
    let directory = tempfile::tempdir().unwrap();
    let (context, log) = recorded_run(&directory);
    RequestAdmission::check(&context, RequestAttempt::First)
        .await
        .unwrap();
    RequestAdmission::check(&context, RequestAttempt::Reattempt)
        .await
        .unwrap();
    ToolExecutionAdmission::check(&context, "swarm", r#"{"op":"summary"}"#)
        .await
        .unwrap();
    let decisions: Vec<Option<String>> = admission_trail(&log)
        .into_iter()
        .map(|(_, decision)| decision)
        .collect();
    assert_eq!(
        decisions,
        [
            Some("model_gate".to_owned()),
            Some("retry_gate".to_owned()),
            Some("tool_gate".to_owned()),
        ]
    );
}

/// The board calls recorded, in memory.
#[derive(Default)]
struct BoardCalls(std::sync::Mutex<Vec<quecto::domain::swarm::BoardOpObservation>>);

impl quecto::application::swarm::ports::BoardOpLog for BoardCalls {
    fn record(&self, observation: quecto::domain::swarm::BoardOpObservation) {
        self.0.lock().unwrap().push(observation);
    }

    /// No summary this test checks is written.
    fn summarize(&self, _summary: quecto::domain::swarm::SwarmRunSummary) {}
}

impl quecto::application::swarm::ports::SessionOpLog for BoardCalls {
    fn dropped(&self, _drops: quecto::application::swarm::dto::DroppedRecords) {}

    fn take_unnoted(&self) -> quecto::application::swarm::dto::DroppedRecords {
        quecto::application::swarm::dto::DroppedRecords::default()
    }
}

/// Answers request `n` with `STEPS[n]`'s tool call, built from the answers
/// the conversation carries, and the last request with the final text; or,
/// when built `queued`, with its fixed responses in order.
struct MockProvider {
    steps: &'static [Step],
    responses: std::sync::Mutex<std::collections::VecDeque<LlmResponse>>,
    failures: std::sync::Mutex<Vec<String>>,
    requests: std::sync::atomic::AtomicUsize,
    refusals: std::sync::Mutex<Vec<String>>,
}

impl std::fmt::Debug for MockProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockProvider").finish_non_exhaustive()
    }
}

impl MockProvider {
    fn new(steps: &'static [Step]) -> Self {
        Self {
            steps,
            responses: std::sync::Mutex::new(std::collections::VecDeque::new()),
            failures: std::sync::Mutex::new(Vec::new()),
            requests: std::sync::atomic::AtomicUsize::new(0),
            refusals: std::sync::Mutex::new(Vec::new()),
        }
    }
    /// A provider answering with `responses` in order (#2279 review L3).
    fn queued(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.into()),
            ..Self::new(&[])
        }
    }
    /// Fails the next request with `error` before answering the rest.
    fn fail_next(&self, error: &str) {
        self.failures.lock().unwrap().push(error.to_owned());
    }
    fn request_count(&self) -> usize {
        self.requests.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// The tool answers that were errors, in order.
    fn refusals(&self) -> Vec<String> {
        self.refusals.lock().unwrap().clone()
    }
}

impl quecto::application::providers::ports::LlmProvider for MockProvider {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "swarm-fake"
    }
    fn chat(
        &self,
        request: quecto::application::providers::ports::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<LlmResponse, quecto::domain::error::DomainError>,
                > + Send
                + '_,
        >,
    > {
        let index = self
            .requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(error) = self.failures.lock().unwrap().pop() {
            return Box::pin(
                async move { Err(quecto::domain::error::DomainError::Provider(error)) },
            );
        }
        if let Some(response) = self.responses.lock().unwrap().pop_front() {
            return Box::pin(async move { Ok(response) });
        }
        let results: Vec<&Message> = request
            .messages
            .iter()
            .filter(|message| {
                message.role == quecto::domain::conversation::value_objects::message::Role::Tool
            })
            .collect();
        if let Some(last) = results.last().filter(|last| last.is_error) {
            self.refusals.lock().unwrap().push(last.content.clone());
        }
        let answers: Vec<serde_json::Value> = results
            .iter()
            .map(|message| serde_json::from_str(&message.content).unwrap_or_default())
            .collect();
        let response = match self.steps.get(index) {
            Some(step) => {
                let (name, arguments) = step(&answers);
                let mut response = text_response("");
                response.content = None;
                response.tool_calls.push(ToolCall {
                    id: format!("swarm-{index}"),
                    name: name.into(),
                    arguments: arguments.to_string(),
                });
                response
            }
            None => text_response("Verified completion at abc"),
        };
        Box::pin(async move { Ok(response) })
    }
}

fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.into()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn test_config(
    provider: Arc<MockProvider>,
    tool_registry: Box<ToolRegistryImpl>,
) -> quecto::application::agent_loop::AgentLoopConfig {
    quecto::application::agent_loop::AgentLoopConfig {
        provider,
        tool_registry,
        model: "test-model".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: String::new(),
        max_context_tokens: 190000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            quecto::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    }
}
