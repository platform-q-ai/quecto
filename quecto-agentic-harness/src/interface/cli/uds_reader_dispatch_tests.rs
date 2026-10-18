use super::*;

#[tokio::test]
async fn clarification_is_not_acknowledged_when_dispatch_queue_is_full() {
    rejected_queue(false).await;
}

#[tokio::test]
async fn clarification_is_not_acknowledged_when_dispatch_queue_is_closed() {
    rejected_queue(true).await;
}

async fn rejected_queue(closed: bool) {
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let (writer, mut replies) = tokio::sync::mpsc::channel(4);
    super::super::uds_ext_protocol::register_client_writer(&registry, 1, writer);
    let (commands, _receiver) = tokio::sync::mpsc::channel(1);
    commands
        .send(ClientMessage::Command(ClientCommand {
            line: "occupied".into(),
            client_id: 1,
            admitted: None,
        }))
        .await
        .unwrap();
    if closed {
        drop(_receiver);
    }
    let session =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
    let cancel = std::sync::Arc::new(std::sync::Mutex::new(
        super::super::uds_cancel::CancelSlot::Idle,
    ));
    let control = super::super::uds_cancel::TurnControl::default();
    let delivery = dispatch(ReaderDispatchCtx {
        line: r#"{"type":"prompt","streamingBehavior":"steer","message":"Approved: use schema v2","ack":"accept","id":"approval-1"}"#.into(),
        cancel_handle: &cancel, turn_control: &control,
        session: &session, registry: &registry, subagent_registry: &None,
        fleet: None, client_id: 1, cmd_tx: &commands,
    });
    let receive = async {
        let line = replies.recv().await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], "approval-1");
        assert_eq!(
            value["success"], false,
            "queue rejection must not be reported as accepted"
        );
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(delivery, receive);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn malformed_steer_admission_does_not_cancel_or_gate_later_work() {
    for line in [
        r#"{"type":"steer"}"#,
        r#"{"type":"prompt","message":3,"streamingBehavior":"steer","ack":"accept"}"#,
        r#"{"type":"steer","message":"bad id","id":3,"ack":"accept"}"#,
        r#"{"type":"steer","message":"old","message":"new","id":"s1","ack":"accept"}"#,
    ] {
        let registry = super::super::uds_ext_protocol::new_client_tool_registry();
        let (commands, mut received) = tokio::sync::mpsc::channel(1);
        let session =
            crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
        let cancel = std::sync::Arc::new(std::sync::Mutex::new(
            super::super::uds_cancel::CancelSlot::Armed(cancel_tx),
        ));
        let control = super::super::uds_cancel::TurnControl::default();
        assert!(
            dispatch(ReaderDispatchCtx {
                line: line.into(),
                cancel_handle: &cancel,
                turn_control: &control,
                session: &session,
                registry: &registry,
                subagent_registry: &None,
                fleet: None,
                client_id: 1,
                cmd_tx: &commands,
            })
            .await
        );
        assert!(
            !control.is_steer_pending(),
            "malformed input gated valid future work: {line}"
        );
        assert!(matches!(
            cancel_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let ClientMessage::Command(command) = received.recv().await.unwrap() else {
            panic!("command expected")
        };
        assert!(matches!(
            super::super::uds::parse_line(&command.line),
            super::super::uds::LineResult::ParseError(_)
        ));
    }
}

struct TestSwarmControl;
impl crate::application::swarm::ports::SwarmRunControl for TestSwarmControl {
    fn coordinator_board(&self) -> crate::application::swarm::ports::CoordinatorBoardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn nudge_watch(&self) {}
    fn apply(
        &self,
        _: crate::domain::swarm::RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<crate::domain::swarm::RunControlReceipt, crate::domain::error::DomainError>,
    > {
        Box::pin(async {
            Ok(crate::domain::swarm::RunControlReceipt {
                budget: None,
                outcome: None,
                reason: None,
                wake_warnings: Vec::new(),
                resume_blockers: Vec::new(),
                wake_allowed: false,
                status: crate::domain::swarm::RunStatus::Paused,
                generation: 42,
            })
        })
    }
}

#[tokio::test]
async fn supervisor_pause_bypasses_full_turn_queue_and_returns_durable_receipt() {
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let (writer, mut replies) = tokio::sync::mpsc::channel(4);
    super::super::uds_ext_protocol::register_client_writer(&registry, 1, writer);
    let (commands, _receiver) = tokio::sync::mpsc::channel(1);
    commands
        .send(ClientMessage::Command(ClientCommand {
            line: "occupied".into(),
            client_id: 1,
            admitted: None,
        }))
        .await
        .unwrap();
    let session =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
    let cancel = std::sync::Arc::new(std::sync::Mutex::new(
        super::super::uds_cancel::CancelSlot::Idle,
    ));
    let control = super::super::uds_cancel::TurnControl::with_swarm_control(Some(
        std::sync::Arc::new(TestSwarmControl),
    ));
    assert!(control.swarm_control.is_some());
    let completed = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        dispatch(ReaderDispatchCtx {
            line: r#"{"type":"swarm_control","action":"pause","id":"pause-42"}"#.into(),
            cancel_handle: &cancel,
            turn_control: &control,
            session: &session,
            registry: &registry,
            subagent_registry: &None,
            fleet: None,
            client_id: 1,
            cmd_tx: &commands,
        }),
    )
    .await;
    assert!(
        completed.is_ok(),
        "pause must bypass saturated command queue"
    );
    let response: serde_json::Value = serde_json::from_str(&replies.recv().await.unwrap()).unwrap();
    assert_eq!(response["id"], "pause-42");
    assert_eq!(response["data"]["status"], "paused");
    assert_eq!(response["data"]["generation"], 42);
}

#[tokio::test]
async fn targeted_pause_must_not_silently_pause_the_receiving_parent() {
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let (writer, mut replies) = tokio::sync::mpsc::channel(4);
    super::super::uds_ext_protocol::register_client_writer(&registry, 1, writer);
    let (commands, _receiver) = tokio::sync::mpsc::channel(1);
    commands
        .send(ClientMessage::Command(ClientCommand {
            line: "occupied".into(),
            client_id: 1,
            admitted: None,
        }))
        .await
        .unwrap();
    let session =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
    let cancel = std::sync::Arc::new(std::sync::Mutex::new(
        super::super::uds_cancel::CancelSlot::Idle,
    ));
    let control = super::super::uds_cancel::TurnControl::with_swarm_control(Some(
        std::sync::Arc::new(TestSwarmControl),
    ));
    assert!(control.swarm_control.is_some());
    let completed = tokio::time::timeout(std::time::Duration::from_millis(250), dispatch(ReaderDispatchCtx {
        line: r#"{"type":"swarm_control","action":"pause","agent_id":"missing-descendant","id":"pause-42"}"#.into(),
        cancel_handle: &cancel, turn_control: &control, session: &session,
        registry: &registry, subagent_registry: &None, fleet: None,
        client_id: 1, cmd_tx: &commands,
    })).await;
    assert!(
        completed.is_ok(),
        "pause must bypass saturated command queue"
    );
    let response: serde_json::Value = serde_json::from_str(&replies.recv().await.unwrap()).unwrap();
    assert_eq!(response["id"], "pause-42");
    assert_eq!(
        response["success"], false,
        "targeted command must not mutate the receiver"
    );
}

/// #1729: `extend` needs positive seconds; `close` reaches the control port
/// below the model like every other supervisor control.
#[tokio::test]
async fn supervisor_extend_requires_seconds_and_close_returns_a_receipt() {
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let (writer, mut replies) = tokio::sync::mpsc::channel(4);
    super::super::uds_ext_protocol::register_client_writer(&registry, 1, writer);
    let (commands, _receiver) = tokio::sync::mpsc::channel(1);
    let session =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
    let cancel = std::sync::Arc::new(std::sync::Mutex::new(
        super::super::uds_cancel::CancelSlot::Idle,
    ));
    let control = super::super::uds_cancel::TurnControl::with_swarm_control(Some(
        std::sync::Arc::new(TestSwarmControl),
    ));
    for (line, expect_error) in [
        (
            r#"{"type":"swarm_control","action":"extend","id":"x1"}"#,
            true,
        ),
        (
            r#"{"type":"swarm_control","action":"extend","id":"x2","deadline_seconds":0}"#,
            true,
        ),
        (
            r#"{"type":"swarm_control","action":"extend","id":"x3","deadline_seconds":600}"#,
            false,
        ),
        (
            r#"{"type":"swarm_control","action":"close","id":"c1"}"#,
            false,
        ),
    ] {
        assert!(
            dispatch(ReaderDispatchCtx {
                line: line.into(),
                cancel_handle: &cancel,
                turn_control: &control,
                session: &session,
                registry: &registry,
                subagent_registry: &None,
                fleet: None,
                client_id: 1,
                cmd_tx: &commands,
            })
            .await
        );
        let response: serde_json::Value =
            serde_json::from_str(&replies.recv().await.unwrap()).unwrap();
        assert_eq!(response["success"], !expect_error, "{line}: {response}");
        if expect_error {
            assert!(
                response["error"]
                    .as_str()
                    .unwrap()
                    .contains("deadline_seconds"),
                "{response}"
            );
        } else {
            assert_eq!(response["data"]["generation"], 42, "{response}");
        }
    }
}

/// A control port whose receipt carries resume blockers (#1924).
struct BlockedSwarmControl;
impl crate::application::swarm::ports::SwarmRunControl for BlockedSwarmControl {
    fn coordinator_board(&self) -> crate::application::swarm::ports::CoordinatorBoardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn nudge_watch(&self) {}
    fn apply(
        &self,
        _: crate::domain::swarm::RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<crate::domain::swarm::RunControlReceipt, crate::domain::error::DomainError>,
    > {
        Box::pin(async {
            Ok(crate::domain::swarm::RunControlReceipt {
                budget: None,
                outcome: Some(crate::domain::swarm::RunStatus::Failed),
                reason: Some("harness exited".into()),
                wake_warnings: Vec::new(),
                resume_blockers: vec![
                    "relaunch the lost coordinator 'member-7' into the retained environment before resuming"
                        .into(),
                ],
                wake_allowed: false,
                status: crate::domain::swarm::RunStatus::Paused,
                generation: 7,
            })
        })
    }
}

#[tokio::test]
async fn swarm_control_status_reply_surfaces_resume_blockers() {
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let (writer, mut replies) = tokio::sync::mpsc::channel(4);
    super::super::uds_ext_protocol::register_client_writer(&registry, 1, writer);
    let (commands, _receiver) = tokio::sync::mpsc::channel(1);
    let session =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
    let cancel = std::sync::Arc::new(std::sync::Mutex::new(
        super::super::uds_cancel::CancelSlot::Idle,
    ));
    let control = super::super::uds_cancel::TurnControl::with_swarm_control(Some(
        std::sync::Arc::new(BlockedSwarmControl),
    ));
    assert!(
        dispatch(ReaderDispatchCtx {
            line: r#"{"type":"swarm_control","action":"status","id":"s1"}"#.into(),
            cancel_handle: &cancel,
            turn_control: &control,
            session: &session,
            registry: &registry,
            subagent_registry: &None,
            fleet: None,
            client_id: 1,
            cmd_tx: &commands,
        })
        .await
    );
    let response: serde_json::Value = serde_json::from_str(&replies.recv().await.unwrap()).unwrap();
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(response["data"]["status"], "paused");
    assert_eq!(response["data"]["outcome"], "failed");
    assert_eq!(
        response["data"]["resume_blockers"],
        serde_json::json!([
            "relaunch the lost coordinator 'member-7' into the retained environment before resuming"
        ]),
        "{response}"
    );
    // The plain receipt (no blockers) carries no key at all.
    let control = super::super::uds_cancel::TurnControl::with_swarm_control(Some(
        std::sync::Arc::new(TestSwarmControl),
    ));
    assert!(
        dispatch(ReaderDispatchCtx {
            line: r#"{"type":"swarm_control","action":"status","id":"s2"}"#.into(),
            cancel_handle: &cancel,
            turn_control: &control,
            session: &session,
            registry: &registry,
            subagent_registry: &None,
            fleet: None,
            client_id: 1,
            cmd_tx: &commands,
        })
        .await
    );
    let response: serde_json::Value = serde_json::from_str(&replies.recv().await.unwrap()).unwrap();
    assert!(
        response["data"].get("resume_blockers").is_none(),
        "{response}"
    );
}

/// Counts the watch nudges and the control actions it was asked for.
#[derive(Default)]
struct WatchedControl {
    nudges: std::sync::atomic::AtomicUsize,
    applied: std::sync::atomic::AtomicUsize,
}
impl crate::application::swarm::ports::SwarmRunControl for WatchedControl {
    fn coordinator_board(&self) -> crate::application::swarm::ports::CoordinatorBoardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
    fn nudge_watch(&self) {
        self.nudges
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn apply(
        &self,
        _: crate::domain::swarm::RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<crate::domain::swarm::RunControlReceipt, crate::domain::error::DomainError>,
    > {
        self.applied
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async {
            Ok(crate::domain::swarm::RunControlReceipt {
                budget: None,
                outcome: None,
                reason: None,
                wake_warnings: Vec::new(),
                resume_blockers: Vec::new(),
                wake_allowed: true,
                status: crate::domain::swarm::RunStatus::Running,
                generation: 7,
            })
        })
    }
}

/// Dispatches `line` with `control` as the swarm port and an empty
/// command channel; answers the reply and what reached the channel.
async fn dispatched_with(
    control: std::sync::Arc<WatchedControl>,
    line: &str,
) -> (
    serde_json::Value,
    super::super::uds_cancel::TurnControl,
    Option<ClientMessage>,
) {
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let (writer, mut replies) = tokio::sync::mpsc::channel(4);
    super::super::uds_ext_protocol::register_client_writer(&registry, 1, writer);
    let (commands, mut received) = tokio::sync::mpsc::channel(4);
    let session =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
    let cancel = std::sync::Arc::new(std::sync::Mutex::new(
        super::super::uds_cancel::CancelSlot::Idle,
    ));
    let turn_control = super::super::uds_cancel::TurnControl::with_swarm_control(Some(control));
    assert!(
        dispatch(ReaderDispatchCtx {
            line: line.into(),
            cancel_handle: &cancel,
            turn_control: &turn_control,
            session: &session,
            registry: &registry,
            subagent_registry: &None,
            fleet: None,
            client_id: 1,
            cmd_tx: &commands,
        })
        .await
    );
    let reply = tokio::time::timeout(std::time::Duration::from_secs(2), replies.recv())
        .await
        .expect("a reply")
        .expect("a reply line");
    (
        serde_json::from_str(&reply).unwrap(),
        turn_control,
        received.try_recv().ok(),
    )
}

/// #2390 (c): a pushed `watch` is accepted on the control allowlist and
/// only nudges this process's run watch: no control action, no model
/// turn, no queued prompt or wake, and the wake cursor is left alone.
#[tokio::test]
async fn a_pushed_watch_only_nudges_the_run_watch() {
    let control = std::sync::Arc::new(WatchedControl::default());
    let (reply, turn_control, queued) = dispatched_with(
        control.clone(),
        r#"{"type":"swarm_control","action":"watch","id":"watch-1"}"#,
    )
    .await;
    assert_eq!(reply["id"], "watch-1");
    assert_eq!(reply["success"], true, "{reply}");
    assert_eq!(control.nudges.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        control.applied.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no control action, so no wake is accepted and no cursor moves"
    );
    assert!(queued.is_none(), "nothing reached the turn queue");
    assert_eq!(turn_control.take_swarm_wake(5), 5, "no wake was queued");
    assert_eq!(turn_control.control_generation(), None);
}

/// #2390 (d): a received `wake` hint also nudges the run watch, as well
/// as queueing its wake.
#[tokio::test]
async fn a_wake_hint_also_nudges_the_run_watch() {
    let control = std::sync::Arc::new(WatchedControl::default());
    let (reply, _turn_control, queued) = dispatched_with(
        control.clone(),
        r#"{"type":"swarm_control","action":"wake","generation":3,"id":"wake-1"}"#,
    )
    .await;
    assert_eq!(reply["success"], true, "{reply}");
    assert_eq!(control.nudges.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        matches!(queued, Some(ClientMessage::SwarmWake { generation: 3 })),
        "the wake itself is still queued"
    );
}

/// #2390 review round 2, L4: only a `wake` whose generation validates
/// nudges the run watch.
#[tokio::test]
async fn a_wake_without_a_generation_nudges_nothing() {
    let control = std::sync::Arc::new(WatchedControl::default());
    let (reply, _turn_control, queued) = dispatched_with(
        control.clone(),
        r#"{"type":"swarm_control","action":"wake","id":"wake-1"}"#,
    )
    .await;
    assert_eq!(reply["success"], false, "{reply}");
    assert_eq!(control.nudges.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(queued.is_none(), "nothing reached the turn queue");
}

#[path = "uds_reader_dispatch_image_tests.rs"]
mod image_tests;
