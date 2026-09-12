//! BDD steps for the owned-child supervisor with real processes and for a
//! real `quecto` child launched by this harness (#1935). Continues
//! `parent_control_steps` (split for the per-file line gate).
use std::sync::Arc;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use quecto::domain::subagent_teardown::ShutdownReason;
use quecto::infrastructure::processes::owned_child_supervisor::{
    ChildExit, ChildHandleId, OwnedChildSupervisor, ProcessGroup, ProtocolOutcome, SentSignal,
    TerminationBudget, TerminationOutcome,
};
use quecto::infrastructure::tools::subagent_registry::SubagentEntry;

use crate::QuectoWorld;
use crate::parent_control_steps::{SupervisorRig, state};

fn fast_budget() -> TerminationBudget {
    TerminationBudget {
        exit_after_ack: Duration::from_millis(300),
        term_grace: Duration::from_millis(500),
        kill_grace: Duration::from_secs(2),
    }
}

// ── Supervisor with real processes ─────────────────────────────────────────

fn rig(world: &mut QuectoWorld) -> &mut SupervisorRig {
    let s = state(world);
    if s.supervisor.is_none() {
        s.supervisor = Some(SupervisorRig {
            runtime: tokio::runtime::Runtime::new().unwrap(),
            supervisor: Arc::new(OwnedChildSupervisor::new()),
            handle: None,
            told_stdin: None,
            outcome: None,
            signals_before_protocol: None,
        });
    }
    s.supervisor.as_mut().unwrap()
}

fn child_command(program: &str, args: &[&str]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd
}

fn adopt(world: &mut QuectoWorld, program: &str, args: &[&str]) {
    let rig = rig(world);
    let handle = rig.runtime.block_on(async {
        rig.supervisor
            .spawn(child_command(program, args), ProcessGroup::Inherited)
            .await
            .expect("spawn child")
            .handle
    });
    rig.handle = Some(handle);
}

#[given("the supervisor adopts a sleeping child")]
fn given_sleeping(world: &mut QuectoWorld) {
    adopt(world, "sleep", &["30"]);
}

#[given("the supervisor adopts a child that ignores TERM")]
fn given_stubborn(world: &mut QuectoWorld) {
    adopt(world, "sh", &["-c", "trap '' TERM; sleep 30 & wait $!"]);
    std::thread::sleep(Duration::from_millis(200));
}

/// A child that exits with 0 as soon as its stdin closes: the "protocol"
/// of this fixture is closing that pipe, so the exit is caused, not timed.
#[given("the supervisor adopts a child that exits when told")]
fn given_told(world: &mut QuectoWorld) {
    let rig = rig(world);
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg("read _; exit 0");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let spawned = rig.runtime.block_on(async {
        rig.supervisor
            .spawn(cmd, ProcessGroup::Inherited)
            .await
            .expect("spawn child")
    });
    rig.handle = Some(spawned.handle);
    rig.told_stdin = spawned.stdin;
}

#[when("the child is terminated with an acknowledged protocol outcome that tells it")]
fn when_terminate_ack_telling(world: &mut QuectoWorld) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    let supervisor = rig.supervisor.clone();
    let stdin = rig.told_stdin.take().expect("the child's stdin");
    let outcome = rig.runtime.block_on(async move {
        supervisor
            .terminate(
                handle,
                async move {
                    // Acknowledge, then let the child exit by itself.
                    drop(stdin);
                    ProtocolOutcome::Acknowledged
                },
                TerminationBudget {
                    exit_after_ack: Duration::from_secs(20),
                    ..fast_budget()
                },
            )
            .await
    });
    rig.outcome = Some(outcome);
}

#[given("the supervisor adopts a child that exits immediately")]
fn given_immediate(world: &mut QuectoWorld) {
    adopt(world, "true", &[]);
}

#[given("the supervisor has observed the child's exit")]
fn given_observed_exit(world: &mut QuectoWorld) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    let exit = rig.runtime.block_on(rig.supervisor.wait_exit(handle));
    assert_eq!(exit, Some(ChildExit::Code(0)));
}

#[when(expr = "the child is terminated with a negative protocol outcome {string}")]
fn when_terminate_negative(world: &mut QuectoWorld, detail: String) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    let supervisor = rig.supervisor.clone();
    let seen = Arc::new(std::sync::Mutex::new(None));
    let seen_in = seen.clone();
    let outcome = rig.runtime.block_on(async move {
        let probe = {
            let supervisor = supervisor.clone();
            async move {
                *seen_in.lock().unwrap() = Some(supervisor.signals_sent(handle));
                ProtocolOutcome::Negative(detail)
            }
        };
        supervisor.terminate(handle, probe, fast_budget()).await
    });
    rig.signals_before_protocol = seen.lock().unwrap().take();
    rig.outcome = Some(outcome);
}

#[when("the child is terminated with an acknowledged protocol outcome")]
fn when_terminate_ack(world: &mut QuectoWorld) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    let supervisor = rig.supervisor.clone();
    let outcome = rig.runtime.block_on(async move {
        supervisor
            .terminate(
                handle,
                async { ProtocolOutcome::Acknowledged },
                TerminationBudget {
                    exit_after_ack: Duration::from_secs(2),
                    ..fast_budget()
                },
            )
            .await
    });
    rig.outcome = Some(outcome);
}

#[then("no signal was sent before the protocol outcome")]
fn then_no_signal_before(world: &mut QuectoWorld) {
    let rig = rig(world);
    assert_eq!(rig.signals_before_protocol.take(), Some(Vec::new()));
}

#[then(expr = "the child exited after TERM authorised by {string}")]
fn then_after_term(world: &mut QuectoWorld, authorised: String) {
    let rig = rig(world);
    match rig.outcome.as_ref().unwrap() {
        TerminationOutcome::ExitedAfterTerm { negative, exit } => {
            assert!(negative.contains(&authorised), "{negative}");
            assert_eq!(*exit, ChildExit::Signal(libc::SIGTERM));
        }
        other => panic!("{other:?}"),
    }
    let handle = rig.handle.unwrap();
    let recorded = rig.supervisor.fallback_authorised_by(handle).unwrap();
    assert!(recorded.contains(&authorised));
}

#[then("the child exited after KILL")]
fn then_after_kill(world: &mut QuectoWorld) {
    let rig = rig(world);
    assert!(
        matches!(
            rig.outcome.as_ref().unwrap(),
            TerminationOutcome::ExitedAfterKill {
                exit: ChildExit::Signal(libc::SIGKILL),
                ..
            }
        ),
        "{:?}",
        rig.outcome
    );
}

#[then("the child exited after the protocol")]
fn then_after_protocol(world: &mut QuectoWorld) {
    let rig = rig(world);
    assert_eq!(
        rig.outcome.as_ref().unwrap(),
        &TerminationOutcome::ExitedAfterProtocol(ChildExit::Code(0))
    );
    assert_eq!(
        rig.supervisor.fallback_authorised_by(rig.handle.unwrap()),
        None
    );
}

#[then("the termination reports the child already exited")]
fn then_already_exited(world: &mut QuectoWorld) {
    let rig = rig(world);
    assert_eq!(
        rig.outcome.as_ref().unwrap(),
        &TerminationOutcome::AlreadyExited(ChildExit::Code(0))
    );
}

#[then(expr = "the supervisor sent exactly {string} to the child")]
fn then_sent_exactly(world: &mut QuectoWorld, expected: String) {
    let rig = rig(world);
    let expected: Vec<SentSignal> = expected
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| match s {
            "TERM" => SentSignal::Term,
            "KILL" => SentSignal::Kill,
            other => panic!("unknown signal {other}"),
        })
        .collect();
    assert_eq!(rig.supervisor.signals_sent(rig.handle.unwrap()), expected);
}

#[then("the supervisor no longer retains the child")]
fn then_not_retained(world: &mut QuectoWorld) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    assert!(!rig.supervisor.retains(handle));
    assert!(rig.supervisor.knows(handle));
}

#[when("a termination is cancelled right after TERM")]
fn when_cancelled_after_term(world: &mut QuectoWorld) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    let supervisor = rig.supervisor.clone();
    rig.runtime.block_on(async move {
        let first = {
            let supervisor = supervisor.clone();
            tokio::spawn(async move {
                supervisor
                    .terminate(
                        handle,
                        async { ProtocolOutcome::Negative("unreachable".into()) },
                        TerminationBudget {
                            term_grace: Duration::from_secs(30),
                            ..fast_budget()
                        },
                    )
                    .await
            })
        };
        while supervisor.signals_sent(handle).is_empty() {
            tokio::task::yield_now().await;
        }
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
    });
}

#[when("two terminations run concurrently with a negative protocol outcome")]
fn when_two_concurrent(world: &mut QuectoWorld) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    let supervisor = rig.supervisor.clone();
    let outcome = rig.runtime.block_on(async move {
        let spawn = |supervisor: Arc<OwnedChildSupervisor>| {
            tokio::spawn(async move {
                supervisor
                    .terminate(
                        handle,
                        async { ProtocolOutcome::Negative("unreachable".into()) },
                        fast_budget(),
                    )
                    .await
            })
        };
        let (a, b) = (spawn(supervisor.clone()), spawn(supervisor));
        let (a, b) = (a.await.unwrap(), b.await.unwrap());
        for outcome in [&a, &b] {
            assert!(
                matches!(
                    outcome,
                    TerminationOutcome::ExitedAfterKill { .. }
                        | TerminationOutcome::AlreadyExited(_)
                ),
                "{outcome:?}"
            );
        }
        a
    });
    rig.outcome = Some(outcome);
}

#[when("a detached termination is requested from a plain thread")]
fn when_detached_from_thread(world: &mut QuectoWorld) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    let supervisor = rig.supervisor.clone();
    std::thread::spawn(move || {
        supervisor.request_termination(
            handle,
            Box::pin(async { ProtocolOutcome::Negative("no socket".into()) }),
            fast_budget(),
        );
    })
    .join()
    .unwrap();
}

#[then("the supervisor reports the child exited by TERM")]
fn then_exited_by_term(world: &mut QuectoWorld) {
    let rig = rig(world);
    let handle = rig.handle.unwrap();
    let exit = rig.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), rig.supervisor.wait_exit(handle))
            .await
            .expect("exit observed")
    });
    assert_eq!(exit, Some(ChildExit::Signal(libc::SIGTERM)));
}

// ── Nothing without a handle is signalled ──────────────────────────────────

fn fixture_pid(world: &mut QuectoWorld) -> u32 {
    state(world).fixture.as_ref().unwrap().id()
}

#[given("a live fixture process that is not owned by the supervisor")]
fn given_fixture_process(world: &mut QuectoWorld) {
    let child = std::process::Command::new("sleep")
        .arg("30")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    state(world).fixture = Some(child);
    let registry = quecto::infrastructure::tools::agent_cmd::AgentCmdTool::new_registry();
    world.agent_cmd_registry = Some(registry);
}

#[given("a registry entry copied from a reaped owned child")]
fn given_copied_entry(world: &mut QuectoWorld) {
    let pid = fixture_pid(world);
    let rig = rig(world);
    let handle = rig.runtime.block_on(async {
        rig.supervisor
            .spawn(child_command("true", &[]), ProcessGroup::Inherited)
            .await
            .expect("spawn child")
            .handle
    });
    rig.runtime.block_on(rig.supervisor.wait_exit(handle));
    let supervisor = rig.supervisor.clone();
    let mut entry = SubagentEntry::new("/tmp/copied.sock".into(), pid);
    entry.owned_child = Some(handle);
    entry.owned_child_supervisor = Some(supervisor);
    let copy = entry.clone();
    world
        .agent_cmd_registry
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .insert("copied".into(), copy);
}

#[given("a restored registry row carrying the fixture pid")]
fn given_restored_row(world: &mut QuectoWorld) {
    let pid = fixture_pid(world);
    let entry = SubagentEntry::new("/tmp/restored.sock".into(), pid);
    world
        .agent_cmd_registry
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .insert("restored".into(), entry);
}

#[given("a container member row carrying the fixture pid")]
fn given_container_row(world: &mut QuectoWorld) {
    let pid = fixture_pid(world);
    let mut entry = SubagentEntry::new("/tmp/member.sock".into(), pid);
    entry.environment_ref = Some("C1".into());
    entry.forwarded_execution_backend = Some("container".into());
    world
        .agent_cmd_registry
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .insert("member".into(), entry);
}

#[when("every legacy teardown path runs against those rows")]
fn when_legacy_paths(world: &mut QuectoWorld) {
    let registry = world.agent_cmd_registry.as_ref().unwrap().clone();
    let entries: Vec<SubagentEntry> = registry.lock().unwrap().values().cloned().collect();
    let mut requests = Vec::new();
    for entry in &entries {
        requests.push(entry.request_owned_child_termination(ShutdownReason::OperatorRequest));
        requests
            .push(quecto::infrastructure::tools::subagent_cascade::terminate_removed_entry(entry));
    }
    quecto::infrastructure::tools::spawn::shutdown_all(&registry);
    state(world).legacy_requests = requests;
    std::thread::sleep(Duration::from_millis(300));
}

#[then("the fixture process is still alive")]
fn then_fixture_alive(world: &mut QuectoWorld) {
    let fixture = state(world).fixture.as_mut().unwrap();
    assert!(
        fixture.try_wait().unwrap().is_none(),
        "the fixture pid must never be signalled"
    );
    let _ = fixture.kill();
    let _ = fixture.wait();
}

#[then("no row requested an owned-child termination")]
fn then_no_requests(world: &mut QuectoWorld) {
    let s = state(world);
    assert!(!s.legacy_requests.is_empty());
    assert!(s.legacy_requests.iter().all(|requested| !requested));
}

#[then("terminating an unknown handle reports no retained handle")]
fn then_unknown_handle(world: &mut QuectoWorld) {
    let rig = rig(world);
    let supervisor = rig.supervisor.clone();
    let ghost = ChildHandleId::probe(u64::MAX);
    assert!(!supervisor.knows(ghost));
    let outcome = rig.runtime.block_on(async move {
        supervisor
            .terminate(
                ghost,
                async { ProtocolOutcome::Negative("x".into()) },
                fast_budget(),
            )
            .await
    });
    assert_eq!(outcome, TerminationOutcome::NoRetainedHandle);
}

#[then("the supervisor sent no signals at all")]
fn then_no_signals_at_all(world: &mut QuectoWorld) {
    let rig = rig(world);
    for handle in rig.supervisor.handles() {
        assert!(rig.supervisor.signals_sent(handle).is_empty());
    }
}

// ── Real child launched by this harness ────────────────────────────────────

fn live_entry(world: &QuectoWorld, agent_id: &str) -> SubagentEntry {
    let registry = world.agent_cmd_registry.as_ref().expect("registry");
    let entries = registry.lock().unwrap();
    entries
        .values()
        .find(|entry| entry.display_name == agent_id)
        .unwrap_or_else(|| panic!("no live entry for {agent_id}"))
        .clone()
}

#[then(expr = "the registry entry for {string} holds an owned child with a launch generation")]
fn then_entry_owned(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    assert!(entry.launch_generation.is_some());
    if !entry.holds_owned_child() {
        let supervisor = entry.owned_child_supervisor.clone().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let exit = runtime.block_on(supervisor.wait_exit(entry.owned_child.unwrap()));
        panic!("the supervisor must retain the handle; child exited with {exit:?}");
    }
    assert!(entry.pid > 0);
}

#[then(expr = "the parent control sidecar for {string} was consumed")]
fn then_sidecar_consumed(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let socket_dir = entry.socket_path.parent().unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(socket_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("quecto-parent-control")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[then(expr = "the child argv for {string} carries --parent-control but no capability material")]
fn then_argv_clean(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let cmdline = std::fs::read(format!("/proc/{}/cmdline", entry.pid)).unwrap();
    let args: Vec<String> = cmdline
        .split(|b| *b == 0)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    assert!(args.iter().any(|a| a == "--parent-control"));
    assert!(args.iter().any(|a| a == "--spawned"));
    assert!(
        !args
            .iter()
            .any(|a| a.len() == 64 && a.bytes().all(|b| b.is_ascii_hexdigit())),
        "capability material on argv: {args:?}"
    );
    // The environment carries none either.
    let environ = std::fs::read(format!("/proc/{}/environ", entry.pid)).unwrap();
    assert!(!String::from_utf8_lossy(&environ).contains("bind_parent_control"));
}

#[when(expr = "the parent drops its control connection to {string}")]
fn when_drop_control_connection(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    entry.monitor_handle.as_ref().expect("monitor task").abort();
}

#[when(expr = "the owned child termination of {string} is requested")]
fn when_request_owned_termination(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let requested = runtime
        .block_on(async { entry.request_owned_child_termination(ShutdownReason::OperatorRequest) });
    assert!(requested, "a locally launched child has an owned handle");
    std::mem::forget(runtime);
}

#[then(expr = "the child process of {string} exits within {int} seconds")]
fn then_child_exits(world: &mut QuectoWorld, agent_id: String, seconds: u64) {
    let entry = live_entry(world, &agent_id);
    // The reaper retires the supervisor slot once it has published the
    // exit, so observe the exit where the registry publishes it.
    let mut exit_rx = entry
        .exit_signal_tx
        .as_ref()
        .expect("exit signal")
        .subscribe();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let exit = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(seconds), async {
            loop {
                if let Some(exit) = exit_rx.borrow().clone() {
                    return exit;
                }
                exit_rx.changed().await.expect("exit signal sender alive");
            }
        })
        .await
    });
    let exit = exit.expect("the child must exit in time");
    assert_eq!(exit.exit_code, Some(0), "a graceful exit, got {exit:?}");
    assert_eq!(exit.signal, None);
    assert!(
        !entry
            .owned_child_supervisor
            .as_ref()
            .unwrap()
            .retains(entry.owned_child.unwrap())
    );
}

#[then(expr = "the supervisor sent no signals to {string}")]
fn then_no_signals_to(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let supervisor = entry.owned_child_supervisor.clone().unwrap();
    assert!(
        supervisor
            .signals_sent(entry.owned_child.unwrap())
            .is_empty()
    );
}

#[then(expr = "the child socket of {string} was removed by a graceful exit")]
fn then_child_socket_removed(world: &mut QuectoWorld, agent_id: String) {
    let entry = live_entry(world, &agent_id);
    let deadline = Instant::now() + Duration::from_secs(5);
    while entry.socket_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!entry.socket_path.exists());
}
