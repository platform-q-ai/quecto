//! Real retained child handle, real monitor framing and sequenced completion.
use super::*;
use crate::infrastructure::processes::owned_child_supervisor::{
    OwnedChildSupervisor, ProcessGroup,
};
use crate::infrastructure::tools::agents::subagent_monitor::{MonitorSpec, spawn_monitor_task};
use crate::infrastructure::tools::agents::subagent_registry::{
    SubagentEntry, SubagentStatus, new_notification_channel,
};
use futures::FutureExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// An exact-test child executable, not a fake notification producer in the
/// parent: bind socket, emit start, block until release, emit end, await EOF.
/// A separate hard child timeout bounds its retained owned process group.
async fn child_server(path: std::path::PathBuf) {
    let listener = tokio::net::UnixListener::bind(path).unwrap();
    println!("RUN16_CHILD_READY");
    let (mut stream, _) = listener.accept().await.unwrap();
    quecto_line_io::write_frame(
        &mut stream,
        br#"{"type":"agent_start"}"#,
        quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
    )
    .await
    .unwrap();
    let mut release = [0_u8; 1];
    use std::io::Read;
    if matches!(std::io::stdin().read_exact(&mut release), Ok(())) {
        quecto_line_io::write_frame(
            &mut stream,
            br#"{"type":"agent_end"}"#,
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
        // Keep the socket alive until the parent has retained the completion.
        let _ = std::io::stdin().read_exact(&mut release);
    }
}

/// Aborting the monitor is unconditional (including assertion panic). The
/// child stdin is likewise owned in scope: EOF ends the finite child server.
struct Monitor(tokio::task::JoinHandle<()>);
impl Drop for Monitor {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn run16_m1_owned_child_completion_wakes_done_then_duplicate_is_inert() {
    if let Some(path) = std::env::var_os("RUN16_CHILD_SERVER") {
        child_server(path.into()).await;
        return;
    }
    let test = concat!(
        module_path!(),
        "::run16_m1_owned_child_completion_wakes_done_then_duplicate_is_inert"
    );
    if isolated(test) {
        let mut replies = done();
        replies.extend([
            Reply::Text("Received the child report."),
            Reply::Text("No declaration."),
        ]);
        let mut rig = Rig::new(replies);
        let path = rig._home.path().join("child.sock");
        let supervisor = OwnedChildSupervisor::process_wide();
        let mut command = tokio::process::Command::new("sh");
        command.args([
            "-c",
            "ulimit -c 0; exec timeout --signal=KILL 15s \"$@\"",
            "run16-child",
        ]);
        command.arg(std::env::current_exe().unwrap());
        let (_, selected) = test.split_once("::").unwrap();
        command.args(["--exact", selected, "--nocapture", "--test-threads=1"]);
        command.env("RUN16_CHILD_SERVER", &path);
        command.stdin(std::process::Stdio::piped());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::null());
        let mut child = supervisor.spawn(command, ProcessGroup::Own).await.unwrap();
        // Keep the readiness pipe open until reap so the child's test harness
        // can write its final bounded result without a broken-pipe failure.
        let mut stdout = tokio::io::BufReader::new(child.stdout.take().unwrap());
        let probe = std::panic::AssertUnwindSafe(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                for _ in 0..10 {
                    let mut line = String::new();
                    assert!(
                        stdout.read_line(&mut line).await.unwrap() > 0,
                        "child readiness before EOF"
                    );
                    if line.contains("RUN16_CHILD_READY") {
                        return;
                    }
                }
                panic!("child readiness marker missing");
            })
            .await
            .expect("bounded child readiness");
            let registry = rig.env.subagent_registry.clone().unwrap();
            let mut entry = SubagentEntry::new(path.clone(), child.display_pid.0);
            entry.owned_child = Some(child.handle);
            entry.owned_child_supervisor = Some(supervisor.clone());
            entry.origin = crate::domain::agents::value_objects::child_end::ChildOrigin::Launched;
            registry.lock().unwrap().insert("owned-child".into(), entry);
            let (notify_tx, mut notify_rx) = new_notification_channel();
            let observer = crate::composition::subagent_lifecycle::build_lifecycle_use_cases(
                registry.clone(),
                None,
                Some(notify_tx.clone()),
                None,
            )
            .observe_exit;
            let mut monitor = Monitor(spawn_monitor_task(MonitorSpec {
                agent_id: "owned-child".into(),
                socket_path: path,
                registry: registry.clone(),
                notify_tx: Some(notify_tx),
                broadcast_tx: Some(rig.tx.clone()),
                parent_id: Some("root".into()),
                parent_control: None,
                observer,
            }));
            tokio::time::timeout(Duration::from_secs(5), async {
                // The actual monitor applies agent_start before broadcasting state.
                loop {
                    let _ = rig.rx.recv().await.unwrap();
                    if registry
                        .lock()
                        .unwrap()
                        .get("owned-child")
                        .is_some_and(|entry| entry.status == SubagentStatus::Running)
                    {
                        break;
                    }
                }
            })
            .await
            .expect("actual owned child observed running");
            assert!(supervisor.retains(child.handle));
            rig.prompt("done-while-child-running").await;
            let initial = rig.frames();
            assert_eq!(
                of_type(&initial, "turn_outcome")[0]["outcome"],
                "done",
                "done is accepted while actual owned child runs"
            );
            assert_eq!(
                registry.lock().unwrap()["owned-child"].status,
                SubagentStatus::Running
            );
            assert!(supervisor.retains(child.handle));
            child.stdin.as_mut().unwrap().write_all(b"x").await.unwrap();
            let note = tokio::time::timeout(Duration::from_secs(5), notify_rx.recv())
                .await
                .expect("monitor completion deadline")
                .expect("real monitor completion channel");
            assert!(
                note.sequence > 0,
                "monitor minted sequence, not unsequenced test prose"
            );
            assert_eq!(
                registry.lock().unwrap()["owned-child"].status,
                SubagentStatus::Idle
            );
            let retained = rig.env.session.enqueue_note(&note);
            assert!(matches!(
                retained,
                crate::interface::cli::uds_session::NotificationEnqueueOutcome::Retained
            ));
            rig.drain().await;
            let requests = rig.provider.requests();
            let frames = rig.frames();
            monitor.0.abort();
            let _ = (&mut monitor.0).await;
            drop(monitor);
            (initial, requests, frames, note)
        })
        .catch_unwind()
        .await;
        // Even setup/channel assertion panics release/terminate and reap the
        // concrete owned process before the panic is propagated.
        drop(child.stdin.take());
        if probe.is_err() {
            use crate::infrastructure::processes::owned_child_supervisor::{
                ProtocolOutcome, TerminationBudget,
            };
            let _ = supervisor
                .terminate(
                    child.handle,
                    async {
                        ProtocolOutcome::Negative("fixture failed after stdin EOF cleanup".into())
                    },
                    TerminationBudget {
                        exit_after_ack: Duration::from_millis(100),
                        term_grace: Duration::from_millis(100),
                        kill_grace: Duration::from_secs(1),
                    },
                )
                .await;
        }
        let exit = tokio::time::timeout(Duration::from_secs(5), supervisor.wait_exit(child.handle))
            .await
            .expect("bounded child cleanup");
        let _ = supervisor.retire(child.handle);
        let (initial, requests, frames, note) = match probe {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        assert!(
            matches!(
                exit,
                Some(crate::infrastructure::processes::owned_child_supervisor::ChildExit::Code(0))
            ),
            "child must exit normally before intended RED assertion: {exit:?}"
        );
        if std::env::var("QUECTO_TURN_OUTCOMES").ok().as_deref() == Some("1") {
            assert_eq!(
                requests.len(),
                2,
                "opt-in settled root waits for next USER despite fresh child report"
            );
            assert!(of_type(&frames, "turn_admitted").is_empty());
            for _ in 0..3 {
                rig.drain().await;
            }
            assert_eq!(rig.provider.requests().len(), 2);
            rig.prompt("next-user").await;
            assert_eq!(
                rig.provider.requests().len(),
                4,
                "next USER receives retained child information and fresh correction"
            );
            let next = rig.frames();
            assert_eq!(of_type(&next, "turn_admitted")[0]["episodeId"], "next-user");
            assert_eq!(of_type(&next, "turn_outcome")[0]["outcome"], "stalled");
            return;
        }
        assert_eq!(
            requests.len(),
            4,
            "actual sequenced child report wakes done root and needs fresh declaration"
        );
        assert!(requests[2].last_user.contains("owned-child"));
        assert_eq!(requests[3].last_user, CORRECTION_NOTE);
        let admitted = of_type(&frames, "turn_admitted");
        assert_eq!(admitted.len(), 2);
        assert_ne!(admitted[0]["episodeId"], "done-while-child-running");
        assert_ne!(
            admitted[0]["turnId"],
            of_type(&initial, "turn_admitted")[0]["turnId"]
        );
        let outcome = of_type(&frames, "turn_outcome");
        assert_eq!(outcome.len(), 1);
        assert_eq!(
            outcome[0]["outcome"], "stalled",
            "old done is not authority for fresh child information"
        );
        assert!(
            outcome[0]["sequence"].as_u64().unwrap()
                > of_type(&initial, "turn_outcome")[0]["sequence"]
                    .as_u64()
                    .unwrap()
        );
        assert!(matches!(
            rig.env.session.enqueue_note(&note),
            crate::interface::cli::uds_session::NotificationEnqueueOutcome::Duplicate
        ));
        for _ in 0..3 {
            rig.drain().await;
        }
        assert_eq!(
            rig.provider.requests().len(),
            4,
            "duplicate sequence and repeated drains do not wake/loop"
        );
        assert!(of_type(&rig.frames(), "turn_outcome").is_empty());
    }
}
