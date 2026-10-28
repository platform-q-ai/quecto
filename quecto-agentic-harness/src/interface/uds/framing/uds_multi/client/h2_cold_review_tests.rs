//! H2 cold review: real guarded producer + first-response-by-id API-style waiter.
//! The provider alone is scripted; transport, dispatcher, registry and session
//! are production composition. Every probe runs in its own hard-killed child.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use super::super::{MultiClientArgs, multi_client_loop};
use crate::application::agent_turn::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::conversation::value_objects::message::{LlmResponse, ToolCall};
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::domain::shared::error::DomainError;

mod run17_ingress_tests;

const STEP: Duration = Duration::from_secs(10);
const PENDING: Duration = Duration::from_millis(200);
const KEY: &str = "cli:h2-cold-review";

#[derive(Clone, Copy, Debug)]
enum Execution {
    Success,
    Failure,
    Cancel,
    CorrectionFailure,
}

#[derive(Debug)]
struct Provider {
    execution: Execution,
    requests: Mutex<usize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

fn reply(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.into()),
        tool_calls: vec![],
        usage: None,
        stop_reason: Some(crate::domain::conversation::value_objects::message::StopReason::EndTurn),
        thinking_blocks: vec![],
    }
}

impl Provider {
    fn result(&self, number: usize) -> Result<LlmResponse, DomainError> {
        match (self.execution, number) {
            (Execution::Failure, _) | (Execution::CorrectionFailure, 2..) => {
                Err(DomainError::Other("H2 actual provider failure".into()))
            }
            (Execution::Success | Execution::Cancel, 1) => {
                let mut response = reply("Answer from actual execution.");
                response.tool_calls.push(ToolCall {
                    id: "declare-h2".into(),
                    name: "declare_turn_outcome".into(),
                    arguments: json!({"outcome":"done", "reason":"Executed H2 fixture"})
                        .to_string(),
                });
                Ok(response)
            }
            (Execution::CorrectionFailure, 1) => Ok(reply("Original successful response.")),
            _ => Ok(reply("")),
        }
    }
}

impl LlmProvider for Provider {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().into()]
    }
    fn name(&self) -> &str {
        "h2-barrier"
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async { panic!("socket fixture must use actual streaming provider path") })
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
        // A finite script: no accidental endless correction/retry success.
        assert!(number <= 3, "unexpected provider retry: {number}");
        let result = self.result(number);
        let (entered, release) = (self.entered.clone(), self.release.clone());
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            tokio::spawn(async move {
                if number == 1 {
                    entered.notify_one();
                    release.notified().await;
                }
                let event = match result {
                    Ok(response) => StreamEvent::Done(response),
                    Err(error) => StreamEvent::Error(error.to_string()),
                };
                let _ = tx.send(event).await;
            });
            rx
        })
    }
}

/// Environment changes are child-local; parent owns scratch cleanup even if
/// timeout kills the child before its TempDir destructor can run.
fn isolated(test: &str, switches: &[&str]) -> bool {
    if std::env::var("QUECTO_RUN16_H2_CASE").ok().as_deref() == Some(test) {
        return true;
    }
    let (_, selection) = test.split_once("::").unwrap();
    let mut failures = Vec::new();
    for switch in switches {
        let scratch = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "ulimit -c 0; exec timeout --signal=KILL 60s \"$@\"",
            "h2-probe",
        ]);
        command.arg(std::env::current_exe().unwrap());
        command.args(["--exact", selection, "--nocapture"]);
        command.env("QUECTO_RUN16_H2_CASE", test);
        command.env("QUECTO_2708_TEST_ROOT", "default");
        command.env("TMPDIR", scratch.path());
        command.env_remove("QUECTO_TURN_OUTCOMES");
        if matches!(*switch, "1" | "other") {
            command.env("QUECTO_TURN_OUTCOMES", switch);
        }
        let output = command.output().unwrap();
        println!(
            "H2 switch={switch} status={}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let selected = String::from_utf8_lossy(&output.stdout).contains("running 1 test\n");
        if matches!((output.status.code(), selected), (Some(0), true)) {
            // Affirmative success; every other process result is evidence failure.
        } else {
            failures.push(format!("{switch}: {} selected={selected}", output.status));
        }
        scratch
            .close()
            .expect("parent cleans probe scratch on every exit");
    }
    assert!(
        failures.is_empty(),
        "H2 isolated cases failed: {failures:?}"
    );
    false
}

type Lines = tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>>;

async fn send(writer: &mut tokio::io::WriteHalf<tokio::net::UnixStream>, value: Value) {
    writer
        .write_all(format!("{value}\n").as_bytes())
        .await
        .unwrap();
    writer.flush().await.unwrap();
}

async fn collect_until(
    lines: &mut Lines,
    frames: &mut Vec<Value>,
    bound: Duration,
    wanted: impl Fn(&Value) -> bool,
) -> bool {
    tokio::time::timeout(bound, async {
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("producer socket open");
            let value: Value = serde_json::from_str(&line).unwrap();
            let found = wanted(&value);
            frames.push(value);
            if found {
                return;
            }
        }
    })
    .await
    .is_ok()
}

fn response(value: &Value, id: &str) -> bool {
    value["type"] == "response" && value["id"] == id
}

struct Observation {
    frames: Vec<Value>,
    early_response: bool,
    requests: usize,
}

/// The first response matching id removes a quecto-api waiter. Its current
/// uncorrelated agent_error fallback can also resolve it; preserve that race
/// in the equivalent consumer rather than pretending a late response fixes it.
fn api_first(frames: &[Value]) -> Option<(usize, &Value)> {
    frames.iter().enumerate().find(|(_, frame)| {
        response(frame, "h2-prompt")
            || (frame["type"] == "response" && frame["command"] == "agent_error")
    })
}

async fn observe(execution: Execution) -> Observation {
    let _signal = crate::interface::uds::test_support::SIMULATED_SIGNAL
        .read()
        .await;
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("h2.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Arc::new(Provider {
        execution,
        requests: Mutex::new(0),
        entered: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
    });
    let (agent_provider, base) = (provider.clone(), dir.path().to_path_buf());
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            use crate::interface::cli::controllers::active_runtime_2708_tests::compose;
            let composed = compose(true);
            assert!(composed.runtime.is_some(), "real production root must be guarded");
            let agent = AgentLoopImpl::new(AgentLoopConfig {
                provider: agent_provider,
                tool_registry: Box::new(composed.registry),
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
                tool_profile_context: crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
            });
            let store = crate::composition::sessions::build_session_handles(
                crate::interface::uds::controllers::uds::dispatch_session_roster_tests::loop_inputs(&base, KEY),
            );
            let catalogue = crate::composition::catalogue::build_catalogue_handles(&base, None);
            let args = MultiClientArgs {
                parent_outcome: composed.runtime,
                extensions: Default::default(),
                agent,
                workspace: &base,
                messages: Vec::new(),
                model: "stub".into(),
                admission_slots: Vec::new(),
                session_key: KEY.into(),
                system_prompt: "system".into(),
                ext_registry: None,
                lifetime: crate::domain::agents::value_objects::harness_lifetime::HarnessLifetime::UntilLastClientDisconnects,
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
            };
            multi_client_loop(args, tokio::net::UnixListener::from_std(listener).unwrap(), &store, &catalogue).await
        })
    });
    let client = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let (reader, mut writer) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(reader).lines();
    send(
        &mut writer,
        json!({"type":"prompt", "id":"h2-prompt", "message":"Execute H2 fixture"}),
    )
    .await;
    tokio::time::timeout(STEP, provider.entered.notified())
        .await
        .expect("actual provider entered");
    let mut frames = Vec::new();
    let early_response = collect_until(&mut lines, &mut frames, PENDING, |f| {
        response(f, "h2-prompt")
    })
    .await;
    match execution {
        Execution::Cancel => send(&mut writer, json!({"type":"abort", "id":"h2-abort"})).await,
        _ => provider.release.notify_one(),
    }
    // Unlike get_state, set_effort is queued behind execution/settlement, so
    // its response is a deterministic end-of-dispatch fence, not a busy read.
    send(
        &mut writer,
        json!({"type":"set_effort", "id":"h2-fence", "effort":"low"}),
    )
    .await;
    let fenced = collect_until(&mut lines, &mut frames, STEP, |f| response(f, "h2-fence")).await;
    provider.release.notify_one();
    drop(writer);
    drop(lines);
    let joined = tokio::task::spawn_blocking(move || thread.join());
    let code = tokio::time::timeout(STEP, joined)
        .await
        .expect("socket loop cleanup bounded")
        .unwrap()
        .unwrap();
    assert_eq!(code, 0, "real producer shuts down cleanly");
    dir.close()
        .expect("socket/session scratch cleaned before behavioral assertions");
    assert!(fenced, "dispatch completed before waiter assertions");
    let requests = *provider.requests.lock().unwrap();
    println!("H2 clean shutdown execution={execution:?} requests={requests} frames={frames:?}");
    Observation {
        frames,
        early_response,
        requests,
    }
}

fn assert_completion(observed: &Observation, success: bool) {
    let (index, first) = api_first(&observed.frames).expect("API waiter resolves");
    assert!(
        response(first, "h2-prompt"),
        "API waiter must resolve by original id, not error fallback: {first}"
    );
    assert_eq!(first["command"], "prompt");
    assert_eq!(
        first["success"], success,
        "actual original execution result at frame {index}"
    );
    assert_eq!(
        observed
            .frames
            .iter()
            .filter(|f| response(f, "h2-prompt"))
            .count(),
        1,
        "one id response only"
    );
    assert!(
        !observed.early_response,
        "default API waiter resolved while actual provider was blocked"
    );
}

#[tokio::test]
async fn h2_default_prompt_response_waits_for_execution_success() {
    if isolated(
        concat!(
            module_path!(),
            "::h2_default_prompt_response_waits_for_execution_success"
        ),
        &["unset", "other"],
    ) {
        let observed = observe(Execution::Success).await;
        assert_completion(&observed, true);
        assert_eq!(
            observed.requests, 2,
            "declaration then empty completion, no correction"
        );
        let response = observed
            .frames
            .iter()
            .position(|f| response(f, "h2-prompt"))
            .unwrap();
        let outcome = observed
            .frames
            .iter()
            .position(|f| f["type"] == "turn_outcome")
            .unwrap();
        let end = observed
            .frames
            .iter()
            .rposition(|f| f["type"] == "agent_end")
            .unwrap();
        assert!(
            end < response && outcome < response,
            "default response follows execution AND guard settlement"
        );
        assert_eq!(observed.frames[outcome]["outcome"], "done");
    }
}

#[tokio::test]
async fn h2_default_prompt_response_reports_execution_failure() {
    if isolated(
        concat!(
            module_path!(),
            "::h2_default_prompt_response_reports_execution_failure"
        ),
        &["unset", "other"],
    ) {
        let observed = observe(Execution::Failure).await;
        assert_completion(&observed, false);
        assert_eq!(observed.requests, 1);
        assert!(
            observed.frames.iter().all(|f| f["type"] != "turn_outcome"),
            "failed execution cannot publish semantic done"
        );
    }
}

#[tokio::test]
async fn h2_default_prompt_cancelled_response_is_correlated_failure() {
    if isolated(
        concat!(
            module_path!(),
            "::h2_default_prompt_cancelled_response_is_correlated_failure"
        ),
        &["unset", "other"],
    ) {
        let observed = observe(Execution::Cancel).await;
        assert_completion(&observed, false);
        assert_eq!(observed.requests, 1);
        assert!(
            observed.frames.iter().all(|f| f["type"] != "turn_outcome"),
            "cancelled execution has no phantom declaration"
        );
    }
}

#[tokio::test]
async fn h2_opt_in_prompt_keeps_admission_ack() {
    if isolated(
        concat!(module_path!(), "::h2_opt_in_prompt_keeps_admission_ack"),
        &["1"],
    ) {
        let observed = observe(Execution::Success).await;
        assert!(
            observed.early_response,
            "env1 Pocket compatibility ACK precedes execution release"
        );
        let (_, first) = api_first(&observed.frames).unwrap();
        assert_eq!(first["success"], true);
        assert!(first["data"]["episodeId"].is_string());
        assert_eq!(
            observed
                .frames
                .iter()
                .filter(|f| response(f, "h2-prompt"))
                .count(),
            1,
            "no duplicate final response"
        );
        assert_eq!(observed.requests, 2);
        let admitted = observed
            .frames
            .iter()
            .position(|f| f["type"] == "turn_admitted")
            .unwrap();
        let ack = observed
            .frames
            .iter()
            .position(|f| response(f, "h2-prompt"))
            .unwrap();
        assert!(ack < admitted, "preserve opt-in ACK ordering");
    }
}

#[tokio::test]
async fn h2_default_correction_response_reports_original_result() {
    if isolated(
        concat!(
            module_path!(),
            "::h2_default_correction_response_reports_original_result"
        ),
        &["unset", "other"],
    ) {
        let observed = observe(Execution::CorrectionFailure).await;
        assert_completion(&observed, true);
        assert_eq!(
            observed.requests, 2,
            "original success then actual correction error"
        );
        assert!(
            observed.frames.iter().all(|f| f["type"] != "turn_outcome"),
            "failed correction cannot invent a declaration"
        );
    }
}
