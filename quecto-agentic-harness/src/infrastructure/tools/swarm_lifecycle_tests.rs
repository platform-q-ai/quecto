use super::*;

#[test]
fn every_non_terminal_run_is_supervised_including_paused() {
    assert!(needs_supervision(RunStatus::Setup));
    assert!(needs_supervision(RunStatus::Running));
    assert!(
        needs_supervision(RunStatus::Paused),
        "a member joining a paused run still needs a watcher"
    );
    for terminal in [
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::Cancelled,
    ] {
        assert!(!needs_supervision(terminal), "{terminal:?}");
    }
}

/// #1715: a supervisor tick records participation from the run status, so a
/// member's shared handle follows the run it watches.
#[test]
fn a_supervisor_tick_records_participation_from_the_run() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    let participation = super::super::swarm_bridge::Participation::shared();
    let mut snapshot = context.snapshot().unwrap();
    assert!(!participation.participating());
    let mut schedule = crate::domain::swarm::watch::WatchSchedule::new();
    let tick =
        super::watch::watch_tick(&context, &mut schedule, &mut snapshot, &participation, 0.0);
    assert_eq!(tick, super::watch::Tick::Snapshot);
    assert!(participation.participating(), "{snapshot:?}");
    assert_eq!(snapshot.status, crate::domain::swarm::RunStatus::Running);
}

/// A member endpoint that answers wake hints and records their generations.
fn wake_sink(
    socket: &std::path::Path,
) -> (
    tokio::task::JoinHandle<Vec<u64>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let listener = tokio::net::UnixListener::bind(socket).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = stop.clone();
    let task = tokio::spawn(async move {
        let mut generations = Vec::new();
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            let accept =
                tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept());
            let Ok(Ok((stream, _))) = accept.await else {
                continue;
            };
            let (read, mut write) = tokio::io::split(stream);
            let mut read = tokio::io::BufReader::new(read);
            let bytes =
                quecto_line_io::read_frame(&mut read, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                    .await
                    .unwrap()
                    .unwrap();
            let command: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(command["type"], "swarm_control");
            assert_eq!(command["action"], "wake");
            generations.push(command["generation"].as_u64().unwrap());
            let response = serde_json::json!({"type":"response","id":command["id"],"success":true});
            quecto_line_io::write_frame(
                &mut write,
                response.to_string().as_bytes(),
                quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
            )
            .await
            .unwrap();
        }
        generations
    });
    (task, stop)
}

/// #1721: a resume is not an actionable board change, so the notification
/// policy targets nobody for it; both resume paths (the control port used by
/// `swarm_control` and the `swarm` tool op) wake every live member with the
/// resume's control generation, so a provider-failure suspension re-arms.
#[tokio::test]
async fn a_resume_wakes_every_live_member_even_though_nothing_targets_them() {
    use crate::application::swarm::ports::SwarmRunControl;
    use crate::domain::swarm::RunControlAction;
    let (directory, context) = crate::swarm_control_fixture::context();
    let socket = directory.path().join("worker.sock");
    let (sink, stop) = wake_sink(&socket);
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("_admit", serde_json::json!(["worker", "r"]))
    })
    .unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call(
            "_activate",
            serde_json::json!(["worker", "r", 123, "identity", socket]),
        )
    })
    .unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.pause("member failed"))
        .unwrap();
    let receipt = context.apply(RunControlAction::Resume).await.unwrap();
    assert_eq!(receipt.status, crate::domain::swarm::RunStatus::Running);
    assert!(receipt.wake_warnings.is_empty(), "{receipt:?}");
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.pause("again")).unwrap();
    // The coordinator's own `swarm resume` op is refused (#1729); only the
    // supervisor's control port resumes, and it wakes everyone.
    let refused =
        super::super::swarm_control::control(context.clone(), "resume", serde_json::json!({}))
            .await
            .expect_err("members cannot resume");
    assert!(
        refused.to_string().contains("outside the swarm"),
        "{refused}"
    );
    let second = context.apply(RunControlAction::Resume).await.unwrap();
    assert!(second.generation > receipt.generation, "{second:?}");
    assert!(second.wake_warnings.is_empty(), "{second:?}");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let generations = sink.await.unwrap();
    assert_eq!(generations, [receipt.generation, second.generation]);
    // A member that cannot be reached is reported, not fatal.
    std::fs::remove_file(&socket).unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.pause("and again"))
        .unwrap();
    let receipt = context.apply(RunControlAction::Resume).await.unwrap();
    assert_eq!(receipt.status, crate::domain::swarm::RunStatus::Running);
    assert_eq!(receipt.wake_warnings.len(), 1, "{receipt:?}");
    assert!(receipt.wake_warnings[0].contains("worker"), "{receipt:?}");
}

#[test]
fn the_settlement_watch_tolerates_a_busy_store_but_not_forever() {
    // #2121: a contended store at close must not end the watch at once, or
    // the member never reaches its self-end and strands the environment.
    let mut budget = super::WatchBudget::default();
    for _ in 1..super::SNAPSHOT_ATTEMPTS {
        assert!(!budget.unreadable_exhausted());
    }
    budget.readable();
    for _ in 1..super::SNAPSHOT_ATTEMPTS {
        assert!(
            !budget.unreadable_exhausted(),
            "a good read resets the count"
        );
    }
    assert!(budget.unreadable_exhausted());
}

#[test]
fn a_member_that_cannot_end_itself_stops_trying_after_a_bounded_number_of_attempts() {
    let mut budget = super::WatchBudget::default();
    for _ in 1..super::SELF_END_ATTEMPTS {
        assert!(!budget.self_end_exhausted());
    }
    assert!(budget.self_end_exhausted());
}

/// Settling a paused run from the swarm tool on an async worker suspends
/// this process's local inference before the settlement returns (#2329
/// final review), on either runtime flavor: the turn is suspended by the
/// time the tool call answers.
async fn settling_a_pause_suspends_local_inference_before_it_returns() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.pause("inspect")).unwrap();
    let suspended = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = suspended.clone();
    let suspend: LocalSuspend = std::sync::Arc::new(move |status, generation| {
        // A suspension that takes a while: it verifies the run on the board.
        std::thread::sleep(std::time::Duration::from_millis(200));
        recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((status, generation));
    });
    super::settle_suspending(context, Some(suspend))
        .await
        .unwrap();
    let suspended = suspended
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(suspended.len(), 1, "{suspended:?}");
    assert_eq!(suspended[0].0, RunStatus::Paused);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_settled_pause_has_suspended_local_inference_on_a_multi_thread_worker() {
    settling_a_pause_suspends_local_inference_before_it_returns().await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_settled_pause_has_suspended_local_inference_on_a_current_thread_runtime() {
    settling_a_pause_suspends_local_inference_before_it_returns().await;
}

/// The event log, in memory: the run summaries written (#2313).
#[derive(Default)]
struct Summaries(std::sync::Mutex<Vec<crate::domain::swarm::SwarmRunSummary>>);

impl crate::application::swarm::ports::BoardOpLog for Summaries {
    fn record(&self, _observation: crate::domain::swarm::BoardOpObservation) {}

    fn summarize(&self, summary: crate::domain::swarm::SwarmRunSummary) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(summary);
    }
}

impl crate::application::swarm::ports::SessionOpLog for Summaries {
    fn dropped(&self, _drops: crate::application::swarm::dto::DroppedRecords) {}

    fn take_unnoted(&self) -> crate::application::swarm::dto::DroppedRecords {
        crate::application::swarm::dto::DroppedRecords::default()
    }
}

impl Summaries {
    fn written(&self) -> Vec<crate::domain::swarm::SwarmRunSummary> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// #2313: the coordinator's harness writes the run's `swarm_run_summary`
/// when it settles a cancelled run, once however often it settles again;
/// a member that is not the coordinator writes none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_coordinators_settlement_writes_the_run_summary_once() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    let summaries = std::sync::Arc::new(Summaries::default());
    assert!(context.board.record_in(summaries.clone()));
    let worker = SwarmContext {
        member: "worker".into(),
        ..context.clone()
    };
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.cancel_run()).unwrap();
    // A stranger's settlement reads the board as no member: refused.
    let _refused = settle(worker).await;
    assert!(
        summaries.written().is_empty(),
        "no member but the coordinator"
    );
    settle(context.clone()).await.unwrap();
    let written = summaries.written();
    assert_eq!(written.len(), 1, "{written:?}");
    assert!(written[0].ops.contains_key("stop"), "{:?}", written[0]);
    // #2313 review M1: the run-wide totals, read from the board at settle.
    let run = written[0].run.as_ref().expect("the board's totals");
    assert!(run.wall_time_us.is_some(), "{run:?}");
    assert!(
        written[0].ops.contains_key("_run_totals"),
        "the read is recorded as the harness's own: {:?}",
        written[0]
    );
    settle(context).await.unwrap();
    assert_eq!(summaries.written().len(), 1, "written once per run");
}

/// A member endpoint that answers every command and records it.
fn command_sink(
    socket: &std::path::Path,
) -> (
    tokio::task::JoinHandle<Vec<serde_json::Value>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let listener = tokio::net::UnixListener::bind(socket).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = stop.clone();
    let task = tokio::spawn(async move {
        let mut commands = Vec::new();
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            let accept =
                tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept());
            let Ok(Ok((stream, _))) = accept.await else {
                continue;
            };
            let (read, mut write) = tokio::io::split(stream);
            let mut read = tokio::io::BufReader::new(read);
            let bytes =
                quecto_line_io::read_frame(&mut read, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                    .await
                    .unwrap()
                    .unwrap();
            let command: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let response = serde_json::json!({"type":"response","id":command["id"],"success":true});
            commands.push(command);
            quecto_line_io::write_frame(
                &mut write,
                response.to_string().as_bytes(),
                quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
            )
            .await
            .unwrap();
        }
        commands
    });
    (task, stop)
}

/// #2390 (b): a control change this process made is pushed to every other
/// live member with an endpoint as a no-turn `watch`, delivered as wake
/// hints are (best effort, a failure a warning); never to itself, a member
/// without an endpoint, or one that is not live.
#[tokio::test]
async fn a_control_change_is_pushed_to_every_other_live_member_as_a_watch() {
    let (directory, context) = crate::swarm_control_fixture::context();
    let worker = directory.path().join("worker.sock");
    let (sink, stop) = command_sink(&worker);
    let own = directory.path().join("own.sock");
    let (own_sink, own_stop) = command_sink(&own);
    let member = |id: &str, status, endpoint: Option<&std::path::Path>| Member {
        id: id.into(),
        status,
        process: None,
        endpoint: endpoint.map(|socket| socket.to_string_lossy().into_owned()),
        launcher: None,
    };
    let gone = directory.path().join("gone.sock");
    let members = [
        member(&context.member, MemberStatus::Live, Some(&own)),
        member("worker", MemberStatus::Live, Some(&worker)),
        member("reserved", MemberStatus::Reserved, Some(&worker)),
        member("dead", MemberStatus::Dead, Some(&worker)),
        member("silent", MemberStatus::Live, None),
    ];
    let warnings = announce_control_change(&context, &members).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let unreachable = [member("lost", MemberStatus::Live, Some(&gone))];
    let warnings = announce_control_change(&context, &unreachable).await;
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("lost"), "{warnings:?}");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    own_stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let commands = sink.await.unwrap();
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert_eq!(commands[0]["type"], "swarm_control");
    assert_eq!(commands[0]["action"], "watch");
    assert!(own_sink.await.unwrap().is_empty(), "never to itself");
}

/// #2390 review L1: the production observer, wired into the watch: a pause
/// this process makes is suspended and pushed, as a `watch`, to the live
/// member's real endpoint; a cancellation another harness pushes ends the
/// watch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_supervisor_pushes_a_local_change_to_a_members_endpoint() {
    let (directory, context) = crate::swarm_control_fixture::context();
    let socket = directory.path().join("worker.sock");
    let (sink, stop) = command_sink(&socket);
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("_admit", serde_json::json!(["worker", "r"]))?;
        context.call(
            "_activate",
            serde_json::json!(["worker", "r", 123, "identity", socket]),
        )
    })
    .unwrap();
    let watching = context.clone();
    let watch = std::thread::spawn(move || {
        let snapshot = watching.snapshot().unwrap();
        watch::watch_until_terminal(
            &watching,
            snapshot,
            &super::super::swarm_bridge::Participation::none(),
            &mut Supervisor(&watching),
        )
        .snapshot
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.pause("operator")).unwrap();
    // The watch reads the pause at once, suspends, then pushes.
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    let other = SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        ..context.clone()
    };
    crate::infrastructure::tools::call_work::off_the_runtime(|| other.cancel_run()).unwrap();
    crate::application::swarm::ports::SwarmRunControl::nudge_watch(&context);
    let ended = tokio::task::spawn_blocking(move || watch.join().unwrap())
        .await
        .unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let commands = sink.await.unwrap();
    assert_eq!(ended.status, RunStatus::Cancelled);
    let actions: Vec<_> = commands
        .iter()
        .map(|command| command["action"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(actions, ["watch"], "{commands:?}");
}
