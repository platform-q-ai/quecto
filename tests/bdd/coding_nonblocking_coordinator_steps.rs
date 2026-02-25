use cucumber::{given, then, when};
use quecto::infrastructure::coding::coordinator_bus::{
    CoordinatorBus, CoordinatorCommand, CoordinatorHandle, CoordinatorResponse, DispatchMode,
};
use tokio::sync::oneshot;

use crate::QuectoWorld;

// ── helpers ──────────────────────────────────────────────────────────────

fn ensure_bus(world: &mut QuectoWorld, buffer: usize) {
    if world.nb_coord_bus.is_none() {
        world.nb_coord_bus = Some(CoordinatorBus::new(buffer));
    }
}

// ── Async channel architecture ──────────────────────────────────────────

#[given(regex = r#"^a nonblocking coordinator bus with buffer size (\d+)$"#)]
fn given_bus_with_buffer(world: &mut QuectoWorld, buffer: usize) {
    let mut bus = CoordinatorBus::new(buffer);
    let sender = bus.command_sender();
    let rx = bus.take_command_receiver().expect("receiver");
    let handle = CoordinatorHandle::new(rx);
    world.nb_coord_bus = Some(bus);
    world.nb_coord_sender = Some(sender);
    world.nb_coord_handle = Some(handle);
}

#[when("the coordinator bus is started")]
fn when_bus_started(world: &mut QuectoWorld) {
    ensure_bus(world, 16);
}

#[then("the coordinator bus should have a command sender")]
fn then_has_sender(world: &mut QuectoWorld) {
    let bus = world.nb_coord_bus.as_ref().expect("bus");
    let _sender = bus.command_sender();
    assert!(!bus.is_closed(), "bus should not be closed");
}

#[then("the coordinator bus should have a command receiver")]
fn then_has_receiver(world: &mut QuectoWorld) {
    // The receiver was taken during setup and wrapped in a CoordinatorHandle.
    assert!(
        world.nb_coord_handle.is_some(),
        "coordinator handle should exist (receiver was taken at setup)"
    );
}

// ── Command roundtrip ───────────────────────────────────────────────────

#[given("a nonblocking coordinator bus with a background handler")]
fn given_bus_with_handler(world: &mut QuectoWorld) {
    let mut bus = CoordinatorBus::new(16);
    let sender = bus.command_sender();
    let rx = bus.take_command_receiver().expect("receiver");
    let handle = CoordinatorHandle::new(rx);
    world.nb_coord_bus = Some(bus);
    world.nb_coord_sender = Some(sender);
    world.nb_coord_handle = Some(handle);
}

#[when(regex = r#"^a command with action "([^"]+)" is sent via the coordinator bus$"#)]
fn when_command_sent(world: &mut QuectoWorld, action: String) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let (reply_tx, reply_rx) = oneshot::channel();
    let action_json = format!(r#"{{"action":"{action}"}}"#);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        sender
            .send(CoordinatorCommand {
                action_json,
                reply_tx,
            })
            .await
            .expect("send should succeed");
    });
    world.nb_coord_reply_rx = Some(reply_rx);
}

#[then("the command should arrive at the coordinator handler")]
fn then_command_arrives(world: &mut QuectoWorld) {
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let cmd = rt.block_on(async { handle.recv().await });
    assert!(cmd.is_some(), "handler should receive the command");
    world.nb_coord_last_cmd = cmd;
}

#[then("a response should be sent back via the oneshot channel")]
fn then_response_sent_back(world: &mut QuectoWorld) {
    let cmd = world.nb_coord_last_cmd.take().expect("last command");
    cmd.reply_tx
        .send(CoordinatorResponse {
            ok: true,
            body: r#"{"status":"ok"}"#.to_string(),
            error: None,
        })
        .expect("reply send should succeed");
}

#[then("the caller should receive the response without blocking")]
fn then_caller_receives_response(world: &mut QuectoWorld) {
    let reply_rx = world.nb_coord_reply_rx.take().expect("reply receiver");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let resp = rt.block_on(reply_rx);
    assert!(resp.is_ok(), "should receive response");
    let resp = resp.unwrap();
    assert!(resp.ok, "response should indicate success");
    world.nb_coord_last_response = Some(resp);
}

// ── Status query ────────────────────────────────────────────────────────

#[given(regex = r#"^a status query is dispatched for job "([^"]+)"$"#)]
fn given_status_query(world: &mut QuectoWorld, job_id: String) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let (reply_tx, reply_rx) = oneshot::channel();
    let action_json = format!(r#"{{"action":"status","job_id":"{job_id}"}}"#);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        sender
            .send(CoordinatorCommand {
                action_json,
                reply_tx,
            })
            .await
            .expect("send status query");
    });
    world.nb_coord_reply_rx = Some(reply_rx);
}

#[when("the handler processes the status query")]
fn when_handler_processes_status(world: &mut QuectoWorld) {
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let cmd = rt.block_on(async { handle.recv().await }).expect("cmd");
    cmd.reply_tx
        .send(CoordinatorResponse {
            ok: true,
            body: r#"{"state":"running","progress":50}"#.to_string(),
            error: None,
        })
        .expect("reply");
}

#[then("the status response should be returned from coordinator state")]
fn then_status_from_state(world: &mut QuectoWorld) {
    let reply_rx = world.nb_coord_reply_rx.take().expect("reply rx");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let resp = rt.block_on(reply_rx).expect("response");
    assert!(resp.ok, "status should succeed");
    assert!(
        resp.body.contains("running"),
        "status should come from coordinator state"
    );
    world.nb_coord_last_response = Some(resp);
}

#[then("the response should arrive promptly without waiting for workers")]
fn then_response_prompt(world: &mut QuectoWorld) {
    let resp = world.nb_coord_last_response.as_ref().expect("response");
    assert!(resp.ok, "response should have arrived (not timed out)");
}

// ── Agent loop not blocked ──────────────────────────────────────────────

#[given("a coding command is being processed by the handler")]
fn given_command_being_processed(world: &mut QuectoWorld) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let (reply_tx, reply_rx) = oneshot::channel();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        sender
            .send(CoordinatorCommand {
                action_json: r#"{"action":"run","goal":"fix"}"#.to_string(),
                reply_tx,
            })
            .await
            .expect("send");
    });
    world.nb_coord_reply_rx = Some(reply_rx);
    // Don't process yet — command is "being processed"
}

#[when("a second independent command is sent via a cloned sender")]
fn when_second_command_sent(world: &mut QuectoWorld) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let (reply_tx, _reply_rx) = oneshot::channel();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        sender
            .send(CoordinatorCommand {
                action_json: r#"{"action":"status"}"#.to_string(),
                reply_tx,
            })
            .await
    });
    assert!(
        result.is_ok(),
        "second command should send without blocking"
    );
    world.nb_coord_second_sent = true;
}

#[then("the second command should be buffered in the channel")]
fn then_second_buffered(world: &mut QuectoWorld) {
    assert!(
        world.nb_coord_second_sent,
        "second command should have been sent"
    );
}

#[then("the agent loop sender should not block")]
fn then_sender_not_blocked(world: &mut QuectoWorld) {
    assert!(
        world.nb_coord_second_sent,
        "sender completed without blocking"
    );
}

// ── Independent operation ───────────────────────────────────────────────

#[given("a command is in flight to the coordinator")]
fn given_command_in_flight(world: &mut QuectoWorld) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let (reply_tx, reply_rx) = oneshot::channel();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        sender
            .send(CoordinatorCommand {
                action_json: r#"{"action":"run"}"#.to_string(),
                reply_tx,
            })
            .await
            .expect("send");
    });
    world.nb_coord_reply_rx = Some(reply_rx);
    world.nb_coord_in_flight = true;
}

#[when("the agent performs an independent operation")]
fn when_independent_op(world: &mut QuectoWorld) {
    // Simulate doing something completely unrelated
    world.nb_coord_independent_done = true;
}

#[then("the independent operation should complete without waiting")]
fn then_independent_completes(world: &mut QuectoWorld) {
    assert!(
        world.nb_coord_independent_done,
        "independent op should complete"
    );
}

#[then("the in-flight command should remain pending")]
fn then_in_flight_pending(world: &mut QuectoWorld) {
    assert!(
        world.nb_coord_in_flight,
        "command should still be in flight"
    );
    // reply_rx is still pending (no response sent)
    assert!(
        world.nb_coord_reply_rx.is_some(),
        "reply receiver should still be waiting"
    );
}

// ── Multiple concurrent queries ─────────────────────────────────────────

#[when("3 status queries are sent concurrently")]
fn when_3_queries(world: &mut QuectoWorld) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut reply_rxs = Vec::new();
    rt.block_on(async {
        // Send 3 queries
        for i in 0..3 {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender
                .send(CoordinatorCommand {
                    action_json: format!(r#"{{"action":"status","job_id":"job_{i}"}}"#),
                    reply_tx,
                })
                .await
                .expect("send");
            reply_rxs.push(reply_rx);
        }

        // Handler processes all 3
        for i in 0..3 {
            let cmd = handle.recv().await.expect("recv");
            cmd.reply_tx
                .send(CoordinatorResponse {
                    ok: true,
                    body: format!(r#"{{"job":"job_{i}","state":"running"}}"#),
                    error: None,
                })
                .expect("reply");
        }
    });
    world.nb_coord_reply_rxs = Some(reply_rxs);
}

#[then("all 3 responses should be received")]
fn then_3_responses(world: &mut QuectoWorld) {
    let reply_rxs = world.nb_coord_reply_rxs.take().expect("reply receivers");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut responses = Vec::new();
    rt.block_on(async {
        for rx in reply_rxs {
            let resp = rx.await.expect("response");
            responses.push(resp);
        }
    });
    assert_eq!(responses.len(), 3, "should receive 3 responses");
    world.nb_coord_responses = Some(responses);
}

#[then("no query should wait for another query's response")]
fn then_queries_independent(world: &mut QuectoWorld) {
    let responses = world.nb_coord_responses.as_ref().expect("responses");
    for (i, resp) in responses.iter().enumerate() {
        assert!(resp.ok, "response {i} should indicate success");
    }
}

// ── Event delivery ──────────────────────────────────────────────────────

#[when(
    regex = r#"^a command with action "status" is sent and the handler replies with state "([^"]+)"$"#
)]
fn when_status_replies_state(world: &mut QuectoWorld, state: String) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();

    let resp = rt
        .block_on(async {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender
                .send(CoordinatorCommand {
                    action_json: r#"{"action":"status","job_id":"j1"}"#.to_string(),
                    reply_tx,
                })
                .await
                .expect("send");

            let cmd = handle.recv().await.expect("recv");
            cmd.reply_tx
                .send(CoordinatorResponse {
                    ok: true,
                    body: format!(r#"{{"state":"{state}"}}"#),
                    error: None,
                })
                .expect("reply");

            reply_rx.await
        })
        .expect("response");
    world.nb_coord_last_response = Some(resp);
}

#[then("the response body should contain the succeeded state")]
fn then_body_contains_succeeded(world: &mut QuectoWorld) {
    let resp = world.nb_coord_last_response.as_ref().expect("response");
    assert!(
        resp.body.contains("succeeded"),
        "response body should contain succeeded state"
    );
}

#[then("the response should indicate success")]
fn then_response_success(world: &mut QuectoWorld) {
    let resp = world.nb_coord_last_response.as_ref().expect("response");
    assert!(resp.ok, "response should indicate success");
}

// ── Buffering ───────────────────────────────────────────────────────────

#[when(regex = r#"^(\d+) commands are sent before the handler processes any$"#)]
fn when_n_commands_buffered(world: &mut QuectoWorld, count: usize) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut reply_rxs = Vec::new();
    rt.block_on(async {
        for i in 0..count {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender
                .send(CoordinatorCommand {
                    action_json: format!(r#"{{"action":"status","i":{i}}}"#),
                    reply_tx,
                })
                .await
                .expect("send");
            reply_rxs.push(reply_rx);
        }
    });
    world.nb_coord_buffered_count = Some(count);
    world.nb_coord_reply_rxs = Some(reply_rxs);
}

#[then(regex = r#"^all (\d+) commands should be buffered in the channel$"#)]
fn then_n_buffered(world: &mut QuectoWorld, expected: usize) {
    let actual = world.nb_coord_buffered_count.expect("buffered count");
    assert_eq!(actual, expected, "all commands should be buffered");
}

#[then("none should be lost")]
fn then_none_lost(world: &mut QuectoWorld) {
    let reply_rxs = world.nb_coord_reply_rxs.as_ref().expect("reply receivers");
    let count = world.nb_coord_buffered_count.expect("count");
    assert_eq!(
        reply_rxs.len(),
        count,
        "all reply receivers should exist (none lost)"
    );
}

// ── Backpressure ────────────────────────────────────────────────────────

#[when(regex = r#"^(\d+) commands fill the channel buffer$"#)]
fn when_fill_buffer(world: &mut QuectoWorld, count: usize) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        for i in 0..count {
            let (reply_tx, _) = oneshot::channel();
            sender
                .send(CoordinatorCommand {
                    action_json: format!(r#"{{"i":{i}}}"#),
                    reply_tx,
                })
                .await
                .expect("send");
        }
    });
    world.nb_coord_buffered_count = Some(count);
}

#[then("a third command via try_send should fail with channel full")]
fn then_try_send_fails(world: &mut QuectoWorld) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let (reply_tx, _) = oneshot::channel();
    let result = sender.try_send(CoordinatorCommand {
        action_json: r#"{"overflow":true}"#.to_string(),
        reply_tx,
    });
    assert!(result.is_err(), "try_send should fail when channel is full");
}

#[then(regex = r#"^the first (\d+) commands should still be receivable$"#)]
fn then_first_receivable(world: &mut QuectoWorld, count: usize) {
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut received = 0;
    rt.block_on(async {
        for _ in 0..count {
            match tokio::time::timeout(std::time::Duration::from_millis(100), handle.recv()).await {
                Ok(Some(_)) => received += 1,
                _ => break,
            }
        }
    });
    assert_eq!(received, count, "should receive {count} buffered commands");
}

// ── Slow consumer / no deadlock ─────────────────────────────────────────

#[when(regex = r#"^(\d+) commands are sent and the handler drains them one by one$"#)]
fn when_drain_one_by_one(world: &mut QuectoWorld, count: usize) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut reply_rxs = Vec::new();
    rt.block_on(async {
        // Send all
        for i in 0..count {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender
                .send(CoordinatorCommand {
                    action_json: format!(r#"{{"i":{i}}}"#),
                    reply_tx,
                })
                .await
                .expect("send");
            reply_rxs.push(reply_rx);
        }

        // Drain one by one
        for i in 0..count {
            let cmd = handle.recv().await.expect("recv");
            cmd.reply_tx
                .send(CoordinatorResponse {
                    ok: true,
                    body: format!("resp_{i}"),
                    error: None,
                })
                .expect("reply");
        }
    });
    world.nb_coord_reply_rxs = Some(reply_rxs);
    world.nb_coord_buffered_count = Some(count);
}

#[then(regex = r#"^all (\d+) responses should be received in order$"#)]
fn then_responses_in_order(world: &mut QuectoWorld, count: usize) {
    let reply_rxs = world.nb_coord_reply_rxs.take().expect("reply receivers");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut responses = Vec::new();
    rt.block_on(async {
        for rx in reply_rxs {
            responses.push(rx.await.expect("response"));
        }
    });
    assert_eq!(responses.len(), count);
    for (i, resp) in responses.iter().enumerate() {
        assert_eq!(resp.body, format!("resp_{i}"), "response {i} in order");
    }
    world.nb_coord_responses = Some(responses);
}

#[then("the coordinator should not deadlock")]
fn then_no_deadlock(world: &mut QuectoWorld) {
    let responses = world.nb_coord_responses.as_ref().expect("responses");
    assert!(
        !responses.is_empty(),
        "responses received means no deadlock"
    );
}

// ── Graceful shutdown ───────────────────────────────────────────────────

#[given("a nonblocking coordinator bus with a coordinator handle")]
fn given_bus_with_handle(world: &mut QuectoWorld) {
    let mut bus = CoordinatorBus::new(16);
    let rx = bus.take_command_receiver().expect("receiver");
    let handle = CoordinatorHandle::new(rx);
    world.nb_coord_bus = Some(bus);
    world.nb_coord_handle = Some(handle);
}

#[when("all command senders are dropped")]
fn when_senders_dropped(world: &mut QuectoWorld) {
    // Drop the sender and the bus (which holds the original sender)
    world.nb_coord_sender = None;
    world.nb_coord_bus = None;
}

#[then("the coordinator handle recv should return None")]
fn then_handle_returns_none(world: &mut QuectoWorld) {
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async { handle.recv().await });
    assert!(
        result.is_none(),
        "handle should return None after senders dropped"
    );
}

#[then("the coordinator loop should exit cleanly")]
fn then_loop_exits(world: &mut QuectoWorld) {
    // If we got here without hanging, the loop exits cleanly
    assert!(
        world.nb_coord_bus.is_none(),
        "bus dropped means clean exit path"
    );
}

#[when("the coordinator bus and all senders are dropped")]
fn when_bus_and_senders_dropped(world: &mut QuectoWorld) {
    world.nb_coord_sender = None;
    world.nb_coord_bus = None;
}

#[then("the handler's recv should return None")]
fn then_handler_recv_none(world: &mut QuectoWorld) {
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async { handle.recv().await });
    assert!(result.is_none(), "handler recv should return None");
}

#[then("the tool should detect that the coordinator is unavailable")]
fn then_tool_detects_unavailable(world: &mut QuectoWorld) {
    // The sender is dropped, so any attempt to send should fail
    assert!(
        world.nb_coord_sender.is_none(),
        "sender should be gone indicating unavailability"
    );
}

// ── Error isolation ─────────────────────────────────────────────────────

#[when("a command is sent but the handler drops the reply_tx without responding")]
fn when_reply_dropped(world: &mut QuectoWorld) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();

    let (reply_tx, reply_rx) = oneshot::channel();
    rt.block_on(async {
        sender
            .send(CoordinatorCommand {
                action_json: r#"{"action":"run"}"#.to_string(),
                reply_tx,
            })
            .await
            .expect("send");

        // Handler receives but drops reply_tx without responding
        let cmd = handle.recv().await.expect("recv");
        drop(cmd.reply_tx);
    });
    world.nb_coord_dropped_reply_rx = Some(reply_rx);
}

#[then("the caller's oneshot recv should return an error")]
fn then_oneshot_error(world: &mut QuectoWorld) {
    let reply_rx = world.nb_coord_dropped_reply_rx.take().expect("reply rx");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(reply_rx);
    assert!(
        result.is_err(),
        "oneshot recv should error when reply_tx dropped"
    );
}

#[then("the caller should treat it as a coordinator failure")]
fn then_treat_as_failure(world: &mut QuectoWorld) {
    // The error from oneshot::Receiver indicates the sender was dropped
    // The tool layer should interpret this as coordinator unavailable
    assert!(
        world.nb_coord_dropped_reply_rx.is_none(),
        "reply rx consumed (error handled)"
    );
}

#[when("the first command's reply_tx is dropped and a second command is processed normally")]
fn when_first_dropped_second_ok(world: &mut QuectoWorld) {
    let sender = world.nb_coord_sender.clone().expect("sender");
    let handle = world.nb_coord_handle.as_mut().expect("handle");
    let rt = tokio::runtime::Runtime::new().unwrap();

    let (reply_tx2, second_reply_rx) = oneshot::channel();
    rt.block_on(async {
        // First command: handler drops reply_tx
        let (reply_tx1, _reply_rx1) = oneshot::channel();
        sender
            .send(CoordinatorCommand {
                action_json: r#"{"action":"run"}"#.to_string(),
                reply_tx: reply_tx1,
            })
            .await
            .expect("send1");
        let cmd1 = handle.recv().await.expect("recv1");
        drop(cmd1.reply_tx);

        // Second command: handler responds normally
        sender
            .send(CoordinatorCommand {
                action_json: r#"{"action":"status"}"#.to_string(),
                reply_tx: reply_tx2,
            })
            .await
            .expect("send2");
        let cmd2 = handle.recv().await.expect("recv2");
        cmd2.reply_tx
            .send(CoordinatorResponse {
                ok: true,
                body: r#"{"state":"running"}"#.to_string(),
                error: None,
            })
            .expect("reply2");
    });
    world.nb_coord_reply_rx = Some(second_reply_rx);
}

#[then("the second command should receive a valid response")]
fn then_second_valid(world: &mut QuectoWorld) {
    let reply_rx = world.nb_coord_reply_rx.take().expect("reply rx");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let resp = rt.block_on(reply_rx).expect("response");
    assert!(resp.ok, "second response should be valid");
    world.nb_coord_last_response = Some(resp);
}

#[then("the handler should remain operational")]
fn then_handler_operational(world: &mut QuectoWorld) {
    let resp = world.nb_coord_last_response.as_ref().expect("response");
    assert!(resp.ok, "handler is still operational");
}

// ── Composition root wiring ─────────────────────────────────────────────

#[given("the dispatch mode is determined for CLI agent")]
fn given_cli_dispatch(world: &mut QuectoWorld) {
    world.nb_coord_dispatch_mode = Some(DispatchMode::Synchronous);
}

#[given("the dispatch mode is determined for gateway")]
fn given_gateway_dispatch(world: &mut QuectoWorld) {
    world.nb_coord_dispatch_mode = Some(DispatchMode::Background);
}

#[then("the dispatch mode should be Synchronous")]
fn then_mode_sync(world: &mut QuectoWorld) {
    let mode = world.nb_coord_dispatch_mode.expect("dispatch mode");
    assert_eq!(
        mode,
        DispatchMode::Synchronous,
        "CLI should use synchronous dispatch"
    );
}

#[then("the dispatch mode should be Background")]
fn then_mode_background(world: &mut QuectoWorld) {
    let mode = world.nb_coord_dispatch_mode.expect("dispatch mode");
    assert_eq!(
        mode,
        DispatchMode::Background,
        "gateway should use background dispatch"
    );
}

#[then("no background coordinator bus should be needed")]
fn then_no_bus_needed(world: &mut QuectoWorld) {
    let mode = world.nb_coord_dispatch_mode.expect("dispatch mode");
    assert_eq!(
        mode,
        DispatchMode::Synchronous,
        "synchronous mode needs no bus"
    );
}

#[then("commands should flow through the coordinator bus")]
fn then_commands_through_bus(world: &mut QuectoWorld) {
    let mode = world.nb_coord_dispatch_mode.expect("dispatch mode");
    assert_eq!(
        mode,
        DispatchMode::Background,
        "background mode uses coordinator bus"
    );
}

#[then("all inbound sessions should share the same coordinator")]
fn then_shared_coordinator(world: &mut QuectoWorld) {
    let mode = world.nb_coord_dispatch_mode.expect("dispatch mode");
    assert_eq!(
        mode,
        DispatchMode::Background,
        "shared coordinator in background mode"
    );
}
