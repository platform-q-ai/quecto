//! Run17: trusted monitor -> sequenced channel -> persistent socket controller.
//! No direct pending enqueue, fabricated notification or runtime admission.
use super::*;
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildExit, OwnedChildSupervisor, ProcessGroup, ProtocolOutcome, TerminationBudget,
};
use crate::infrastructure::tools::agents::subagent_monitor::{MonitorSpec, spawn_monitor_task};
use crate::infrastructure::tools::agents::subagent_registry::{
    SubagentEntry, SubagentStatus, new_notification_channel,
};
use futures::FutureExt;

#[derive(Clone, Copy, Debug)]
enum Case {
    IdleDone,
    BufferedDone,
    NeedsUser,
}

#[derive(Debug)]
struct IngressProvider {
    case: Case,
    requests: Mutex<Vec<String>>,
    entered: Notify,
    release: Notify,
}
impl LlmProvider for IngressProvider {
    fn name(&self) -> &str {
        "run17-finite-ingress"
    }
    fn route_order(&self) -> Vec<String> {
        vec![self.name().into()]
    }
    fn chat<'a>(
        &'a self,
        _: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async { panic!("real socket path must stream") })
    }
    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        let number = {
            let mut seen = self.requests.lock().unwrap();
            seen.push(
                request
                    .messages
                    .iter()
                    .rev()
                    .find(|m| {
                        m.role == crate::domain::conversation::value_objects::message::Role::User
                    })
                    .map(|m| m.content.clone())
                    .unwrap_or_default(),
            );
            seen.len()
        };
        Box::pin(async move {
            if matches!((self.case, number), (Case::BufferedDone, 2)) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            let mut result = reply(if number == 3 {
                "Fresh child information received."
            } else {
                ""
            });
            if number == 1 {
                result.tool_calls.push(ToolCall {
                    id: "run17-declare".into(), name: "declare_turn_outcome".into(),
                    arguments: json!({"outcome":if matches!(self.case, Case::NeedsUser) {"asking"} else {"done"},"reason":"Initial parent settlement"}).to_string(),
                });
            }
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let event = if number <= 4 {
                StreamEvent::Done(result)
            } else {
                StreamEvent::Error("finite run17 script exhausted".into())
            };
            tx.send(event).await.unwrap();
            rx
        })
    }
}

/// The only fake is a finite child executable speaking real monitor framing.
/// Its supervisor-owned stdin releases completion, then EOF ends the process.
async fn child_server(path: std::path::PathBuf) {
    let listener = tokio::net::UnixListener::bind(path).unwrap();
    println!("RUN17_READY");
    let (mut stream, _) = listener.accept().await.unwrap();
    for event in [
        br#"{"type":"agent_start"}"#.as_slice(),
        br#"{"type":"agent_end"}"#.as_slice(),
    ] {
        if event == br#"{"type":"agent_end"}"# {
            let mut byte = [0];
            use std::io::Read;
            if std::io::stdin().read_exact(&mut byte).is_err() {
                return;
            }
        }
        quecto_line_io::write_frame(&mut stream, event, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap();
    }
    let mut byte = [0];
    use std::io::Read;
    let _ = std::io::stdin().read_exact(&mut byte);
}

async fn observe(case: Case, selection: &str) {
    let _signal = crate::interface::uds::test_support::SIMULATED_SIGNAL
        .read()
        .await;
    let dir = tempfile::TempDir::new_in("/tmp").unwrap();
    let child_path = dir.path().join("c.sock");
    let parent_path = dir.path().join("p.sock");
    let composed = crate::interface::cli::controllers::active_runtime_2708_tests::compose(true);
    let registry = composed
        .subagents
        .clone()
        .expect("production spawn registry");
    // One channel, one receiver: monitor owns producer, multi_client_loop owns
    // consumer. The builder's unused spawn sender is NOT a producer here.
    let (notify_tx, notify_rx) = new_notification_channel();
    let (broadcast, _) = tokio::sync::broadcast::channel(256);
    let mut monitor_events = broadcast.subscribe();
    let supervisor = OwnedChildSupervisor::process_wide();
    let mut command = tokio::process::Command::new("sh");
    command.args([
        "-c",
        "ulimit -c 0; exec timeout --signal=KILL 20s \"$@\"",
        "run17-child",
    ]);
    command.arg(std::env::current_exe().unwrap());
    command.args(["--exact", selection, "--nocapture", "--test-threads=1"]);
    command.env("RUN17_CHILD_SERVER", &child_path);
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = supervisor.spawn(command, ProcessGroup::Own).await.unwrap();
    let mut stdout = tokio::io::BufReader::new(child.stdout.take().unwrap());
    let mut monitor = None;
    let mut loop_thread = None;
    let provider = Arc::new(IngressProvider {
        case,
        requests: Mutex::new(vec![]),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let probe = std::panic::AssertUnwindSafe(async {
        tokio::time::timeout(STEP, async {
            for _ in 0..10 {
                let mut line = String::new();
                assert!(stdout.read_line(&mut line).await.unwrap() > 0);
                if line.contains("RUN17_READY") { return; }
            }
            panic!("child readiness missing");
        }).await.expect("bounded readiness");
        let mut entry = SubagentEntry::new(child_path.clone(), child.display_pid.0);
        entry.owned_child = Some(child.handle);
        entry.owned_child_supervisor = Some(supervisor.clone());
        entry.origin = crate::domain::agents::value_objects::child_end::ChildOrigin::Launched;
        registry.lock().unwrap().insert("run17-child".into(), entry);
        let observer = crate::composition::subagent_lifecycle::build_lifecycle_use_cases(registry.clone(), None, Some(notify_tx.clone()), None).observe_exit;
        monitor = Some(spawn_monitor_task(MonitorSpec { agent_id: "run17-child".into(), socket_path: child_path, registry: registry.clone(), notify_tx: Some(notify_tx), broadcast_tx: Some(broadcast.clone()), parent_id: Some("root".into()), parent_control: None, observer }));
        tokio::time::timeout(STEP, async {
            loop {
                monitor_events.recv().await.unwrap();
                if registry.lock().unwrap()["run17-child"].status == SubagentStatus::Running { break; }
            }
        }).await.expect("real monitor running barrier");
        assert!(supervisor.retains(child.handle));
        let listener = std::os::unix::net::UnixListener::bind(&parent_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (base, loop_provider, loop_registry) = (dir.path().to_path_buf(), provider.clone(), registry.clone());
        loop_thread = Some(std::thread::spawn(move || {
            tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap().block_on(async move {
                let agent = AgentLoopImpl::new(AgentLoopConfig { provider: loop_provider, tool_registry: Box::new(composed.registry), model: "stub".into(), max_tokens:32, temperature:0.0, retention:None, session_key:KEY.into(), max_context_tokens:190_000, progress_callback:None, streaming:true, effort:None, audit_log:None, pin_recent_turns:2, context_marks:Default::default(), model_context_window:None, tool_profile_context:crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent });
                let store = crate::composition::sessions::build_session_handles(crate::interface::uds::controllers::uds::dispatch_session_roster_tests::loop_inputs(&base, KEY));
                let catalogue = crate::composition::catalogue::build_catalogue_handles(&base, None);
                let args = MultiClientArgs { agent, workspace:&base, messages:vec![], model:"stub".into(), admission_slots:vec![], session_key:KEY.into(), system_prompt:composed.snippets, ext_registry:None, lifetime:crate::domain::agents::value_objects::harness_lifetime::HarnessLifetime::UntilLastClientDisconnects, notification_rx:Some(notify_rx), subagent_registry:Some(loop_registry), harness_lifecycle:None, workflow_state:None, workflow_config:None, broadcast_tx:Some(broadcast), parent_control:None, teardown_graph:None, turn_session:crate::composition::bootstrap::agent_turn::build_turn_session, environment_control:None, parent_outcome:composed.runtime, extensions:Default::default() };
                let code = multi_client_loop(args, tokio::net::UnixListener::from_std(listener).unwrap(), &store, &catalogue).await;
                drop(composed._home);
                code
            })
        }));
        let stream = tokio::net::UnixStream::connect(&parent_path).await.unwrap();
        let (reader, mut writer) = tokio::io::split(stream);
        let mut lines = tokio::io::BufReader::new(reader).lines();
        let mut frames = vec![];
        send(&mut writer, json!({"type":"prompt","id":"initial","message":"Settle initial turn"})).await;
        if matches!(case, Case::BufferedDone) {
            tokio::time::timeout(STEP, provider.entered.notified()).await.expect("active turn second provider barrier");
        } else {
            assert!(collect_until(&mut lines, &mut frames, STEP, |f| f["type"] == "turn_outcome").await);
            send(&mut writer, json!({"type":"set_effort","id":"idle-fence","effort":"low"})).await;
            assert!(collect_until(&mut lines, &mut frames, STEP, |f| response(f,"idle-fence")).await);
        }
        child.stdin.as_mut().unwrap().write_all(b"x").await.unwrap();
        // monitor broadcasts AFTER apply_and_notify's sequenced send. This is
        // an enqueue barrier, not a parent-side receive/drain of the channel.
        tokio::time::timeout(STEP, async {
            loop {
                monitor_events.recv().await.unwrap();
                if registry.lock().unwrap()["run17-child"].status == SubagentStatus::Idle { break; }
            }
        }).await.expect("monitor completion enqueued before release");
        let buffered_count = provider.requests.lock().unwrap().len();
        provider.release.notify_one();
        let woke = collect_until(&mut lines, &mut frames, Duration::from_secs(2), |f| f["type"] == "turn_outcome" && f["outcome"] == "stalled").await;
        send(&mut writer, json!({"type":"set_effort","id":"final-fence","effort":"low"})).await;
        assert!(collect_until(&mut lines, &mut frames, STEP, |f| response(f,"final-fence")).await);
        (frames, woke, buffered_count)
    }).catch_unwind().await;
    // All failure paths release provider, drop socket (probe scope), abort and
    // join monitor, release/reap/retire real child and join control loop BEFORE
    // either propagating setup panic or making behavioral assertions.
    provider.release.notify_one();
    if let Some(task) = monitor {
        task.abort();
        let _ = task.await;
    }
    drop(child.stdin.take());
    if matches!(probe, Err(_)) {
        let _ = supervisor
            .terminate(
                child.handle,
                async { ProtocolOutcome::Negative("probe cleanup".into()) },
                TerminationBudget {
                    exit_after_ack: Duration::from_millis(100),
                    term_grace: Duration::from_millis(100),
                    kill_grace: Duration::from_secs(1),
                },
            )
            .await;
    }
    let exit = tokio::time::timeout(STEP, supervisor.wait_exit(child.handle)).await;
    let _ = supervisor.retire(child.handle);
    let code = if let Some(thread) = loop_thread {
        Some(tokio::time::timeout(STEP, tokio::task::spawn_blocking(move || thread.join())).await)
    } else {
        None
    };
    let requests = provider.requests.lock().unwrap().clone();
    dir.close()
        .expect("sockets and scratch removed before assertions");
    let (frames, woke, buffered_count) = match probe {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    assert!(
        matches!(exit, Ok(Some(ChildExit::Code(0)))),
        "child reaped normally: {exit:?}"
    );
    assert!(
        matches!(code, Some(Ok(Ok(Ok(0))))),
        "persistent loop joined cleanly: {code:?}"
    );
    println!(
        "RUN17 teardown complete case={case:?} requests={} buffered_count={buffered_count} woke={woke} frames={frames:?}",
        requests.len()
    );
    if matches!(case, Case::BufferedDone) {
        assert_eq!(buffered_count, 2, "no mid-turn delivery");
    }
    assert_eq!(
        requests.len(),
        4,
        "real monitor/channel idle ingress must wake terminal parent and require fresh declaration"
    );
    assert!(
        woke,
        "fresh report without declaration receives exactly one correction then stalled"
    );
    assert!(requests[2].contains("run17-child"));
    assert_eq!(
        requests[3],
        crate::domain::agents::value_objects::turn_outcome::CORRECTION_NOTE
    );
    let outcomes: Vec<_> = frames
        .iter()
        .filter(|f| f["type"] == "turn_outcome")
        .collect();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        outcomes[0]["outcome"],
        if matches!(case, Case::NeedsUser) {
            "needs_user"
        } else {
            "done"
        }
    );
    assert_eq!(outcomes[1]["outcome"], "stalled");
    assert_ne!(outcomes[0]["episodeId"], outcomes[1]["episodeId"]);
    assert_ne!(outcomes[0]["turnId"], outcomes[1]["turnId"]);
    assert!(outcomes[1]["sequence"].as_u64().unwrap() > outcomes[0]["sequence"].as_u64().unwrap());
}

async fn run(case: Case, name: &str) {
    if let Some(path) = std::env::var_os("RUN17_CHILD_SERVER") {
        child_server(path.into()).await;
        return;
    }
    if isolated(name, &["unset"]) {
        let (_, selection) = name.split_once("::").unwrap();
        observe(case, selection).await;
    }
}
#[tokio::test]
async fn run17_m1_idle_owned_child_completion_wakes_done() {
    run(
        Case::IdleDone,
        concat!(
            module_path!(),
            "::run17_m1_idle_owned_child_completion_wakes_done"
        ),
    )
    .await;
}
#[tokio::test]
async fn run17_m1_owned_child_completion_buffered_during_done_turn() {
    run(
        Case::BufferedDone,
        concat!(
            module_path!(),
            "::run17_m1_owned_child_completion_buffered_during_done_turn"
        ),
    )
    .await;
}
#[tokio::test]
async fn run17_m1_idle_child_report_wakes_needs_user() {
    run(
        Case::NeedsUser,
        concat!(
            module_path!(),
            "::run17_m1_idle_child_report_wakes_needs_user"
        ),
    )
    .await;
}
