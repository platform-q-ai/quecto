use super::*;

fn core_names() -> HashSet<String> {
    ["bash", "read", "write", "edit", "ls", "grep", "find"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn tool_reg(name: &str, desc: &str) -> ToolRegistration {
    ToolRegistration {
        name: name.into(),
        description: desc.into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
    }
}

fn reg(
    registry: &ClientToolRegistry,
    core: &HashSet<String>,
    cid: u64,
    tools: &[ToolRegistration],
) -> (
    bool,
    AgentEvent,
    Vec<std::sync::Arc<dyn crate::domain::tool::Tool>>,
) {
    handle_register_tools(RegisterToolsArgs {
        client_id: cid,
        id: None,
        tools,
        registry,
        core_tool_names: core,
    })
}

/// Test context for register operations.
struct RegCtx {
    registry: ClientToolRegistry,
    core: HashSet<String>,
}

impl RegCtx {
    fn new() -> Self {
        Self {
            registry: new_client_tool_registry(),
            core: core_names(),
        }
    }

    fn register_id(
        &self,
        cid: u64,
        id: &str,
        tools: &[ToolRegistration],
    ) -> (
        bool,
        AgentEvent,
        Vec<std::sync::Arc<dyn crate::domain::tool::Tool>>,
    ) {
        handle_register_tools(RegisterToolsArgs {
            client_id: cid,
            id: Some(id),
            tools,
            registry: &self.registry,
            core_tool_names: &self.core,
        })
    }
}

#[test]
fn test_register_tools_success() {
    let ctx = RegCtx::new();
    let (ok, ev, tools) = ctx.register_id(1, "rt-1", &[tool_reg("weather", "Get weather")]);
    assert!(ok);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].definition().name.as_ref(), "weather");
    assert!(ev.to_json_line().contains("\"success\":true"));
}

#[test]
fn test_register_tools_rejects_core_shadow() {
    let ctx = RegCtx::new();
    let (ok, ev, tools) = ctx.register_id(1, "rt-2", &[tool_reg("bash", "Shadow bash")]);
    assert!(!ok);
    assert!(tools.is_empty());
    let json = ev.to_json_line();
    assert!(json.contains("\"success\":false"));
    assert!(json.contains("shadows a core tool"));
}

#[test]
fn test_register_tools_multiple() {
    let r = new_client_tool_registry();
    let c = core_names();
    let tools_arr = [tool_reg("weather", "W"), tool_reg("translate", "T")];
    let (ok, _, tools) = reg(&r, &c, 1, &tools_arr);
    assert!(ok);
    assert_eq!(tools.len(), 2);
}

#[test]
fn test_register_tools_idempotent() {
    let r = new_client_tool_registry();
    let c = core_names();
    let (ok, _, _) = reg(&r, &c, 1, &[tool_reg("weather", "Old desc")]);
    assert!(ok);
    let (ok, _, tools) = reg(&r, &c, 1, &[tool_reg("weather", "New desc")]);
    assert!(ok);
    assert_eq!(tools[0].definition().description.as_ref(), "New desc");
    assert_eq!(r.lock().unwrap()[&1].tool_names.len(), 1);
}

#[test]
fn test_register_tools_rejects_duplicate_owner() {
    let r = new_client_tool_registry();
    let c = core_names();
    let (ok, _, _) = reg(&r, &c, 1, &[tool_reg("weather", "Owned by one")]);
    assert!(ok);

    let (ok, ev, tools) = reg(&r, &c, 2, &[tool_reg("weather", "Hijack")]);
    assert!(!ok);
    assert!(tools.is_empty());
    let json = ev.to_json_line();
    assert!(json.contains("already registered by client"));
    assert!(r.lock().unwrap()[&1].tool_names.contains("weather"));
    assert!(!r.lock().unwrap().contains_key(&2));
}

#[test]
fn test_unregister_tools() {
    let r = new_client_tool_registry();
    let c = core_names();
    reg(&r, &c, 1, &[tool_reg("weather", "Get weather")]);
    let (ev, removed) = handle_unregister_tools(1, Some("ut-1"), &["weather".into()], &r);
    assert_eq!(removed, vec!["weather"]);
    assert!(ev.to_json_line().contains("\"success\":true"));
    assert!(r.lock().unwrap()[&1].tool_names.is_empty());
}

#[test]
fn test_unregister_tools_unknown_name_is_noop() {
    let r = new_client_tool_registry();
    let (_, removed) = handle_unregister_tools(1, None, &["nonexistent".into()], &r);
    assert!(removed.is_empty());
}

#[test]
fn test_client_disconnect_removes_tools() {
    let r = new_client_tool_registry();
    let c = core_names();
    let tools_arr = [tool_reg("weather", "W"), tool_reg("translate", "T")];
    reg(&r, &c, 1, &tools_arr);
    let removed = handle_client_disconnect(1, &r);
    assert_eq!(removed.len(), 2);
    assert!(!r.lock().unwrap().contains_key(&1));
}

#[test]
fn test_client_disconnect_cancels_pending() {
    let r = new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    {
        r.lock().unwrap().entry(1).or_default().insert_pending(
            "call-1".into(),
            tx,
            std::time::Duration::from_secs(30),
        );
    }
    handle_client_disconnect(1, &r);
    let result = rx.try_recv().unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("disconnected"));
}

#[test]
fn test_handle_tool_result_delivers() {
    let r = new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    {
        r.lock().unwrap().entry(1).or_default().insert_pending(
            "call-1".into(),
            tx,
            std::time::Duration::from_secs(30),
        );
    }
    handle_tool_result(ToolResultArgs {
        client_id: 1,
        tool_call_id: "call-1",
        content: "22°C, sunny",
        is_error: false,
        registry: &r,
    });
    let result = rx.try_recv().unwrap();
    assert_eq!(result.content, "22°C, sunny");
    assert!(!result.is_error);
}

#[test]
fn test_handle_tool_result_unknown_call_id_is_noop() {
    let r = new_client_tool_registry();
    handle_tool_result(ToolResultArgs {
        client_id: 1,
        tool_call_id: "nonexistent",
        content: "data",
        is_error: false,
        registry: &r,
    });
}

#[test]
fn test_register_tools_rejects_duplicate_in_same_request() {
    let ctx = RegCtx::new();
    let (ok, ev, tools) = ctx.register_id(
        1,
        "rt-dup",
        &[tool_reg("weather", "First"), tool_reg("weather", "Second")],
    );
    assert!(!ok);
    assert!(tools.is_empty());
    let json = ev.to_json_line();
    assert!(json.contains("\"success\":false"));
    assert!(json.contains("registered more than once"));
}

#[test]
fn test_register_client_writer_sets_writer_tx() {
    let r = new_client_tool_registry();
    let (tx, _rx) = tokio::sync::mpsc::channel::<String>(4);
    register_client_writer(&r, 99, tx);
    let locked = r.lock().unwrap();
    assert!(locked[&99].writer_tx.is_some());
}

#[test]
fn test_client_disconnect_unknown_client_returns_empty() {
    let r = new_client_tool_registry();
    let removed = handle_client_disconnect(404, &r);
    assert!(removed.is_empty());
}

#[test]
fn test_two_clients_different_tools() {
    let r = new_client_tool_registry();
    let c = core_names();
    reg(&r, &c, 1, &[tool_reg("weather", "W")]);
    reg(&r, &c, 2, &[tool_reg("translate", "T")]);
    let locked = r.lock().unwrap();
    assert!(locked[&1].tool_names.contains("weather"));
    assert!(locked[&2].tool_names.contains("translate"));
}

// ─── forward_tool_requests cleanup-on-delivery-failure (V3) ────────────

/// `insert_pending` sweeps entries whose deadline has already passed. A stale
/// entry (its caller long since unblocked by `UdsTool::execute`'s own timeout)
/// must not linger in `pending_results` once a fresh call arrives — the lazy
/// sweep reclaims the map slot without a per-call spawned timer task (#996).
#[test]
fn insert_pending_sweeps_expired_entries() {
    use crate::interface::cli::uds_ext_protocol::PendingResult;

    let registry = new_client_tool_registry();
    let client_id = 55;
    {
        let mut reg = registry.lock().unwrap();
        let state = reg.entry(client_id).or_default();
        // A stale entry whose deadline is already in the past.
        let (stale_tx, _stale_rx) = tokio::sync::oneshot::channel();
        state.pending_results.insert(
            "stale-call".into(),
            PendingResult {
                reply: stale_tx,
                deadline: std::time::Instant::now() - std::time::Duration::from_secs(1),
            },
        );

        // Inserting a fresh, live entry sweeps the expired one.
        let (fresh_tx, _fresh_rx) = tokio::sync::oneshot::channel();
        state.insert_pending(
            "fresh-call".into(),
            fresh_tx,
            std::time::Duration::from_secs(30),
        );

        assert!(
            !state.pending_results.contains_key("stale-call"),
            "expired entry must be swept on the next insert"
        );
        assert!(
            state.pending_results.contains_key("fresh-call"),
            "the freshly inserted live entry must remain"
        );
    }
}

#[tokio::test]
async fn forwarder_cleans_pending_when_writer_has_no_receiver() {
    use crate::domain::extension_tool::ToolInvocation;
    use std::time::Duration;

    let registry = new_client_tool_registry();
    let client_id: u64 = 42;
    {
        let mut reg = registry.lock().unwrap();
        reg.insert(client_id, ClientToolState::default());
    }

    // Writer channel with no receiver — every .send() errs.
    let (writer_tx, writer_rx) = tokio::sync::mpsc::channel::<String>(8);
    drop(writer_rx);

    let (req_tx, req_rx) = tokio::sync::mpsc::channel::<ToolInvocation>(4);
    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<&'static str>();
    let forwarder = tokio::spawn(super::forward_tool_requests(
        client_id,
        "weather".to_string(),
        req_rx,
        shutdown_rx,
        registry.clone(),
        Some(writer_tx),
    ));

    let (reply_tx, result_rx) = tokio::sync::oneshot::channel();
    req_tx
        .send(ToolInvocation {
            tool_call_id: "uds-test-1".into(),
            tool_name: "weather".into(),
            arguments: "{}".into(),
            reply: reply_tx,
        })
        .await
        .unwrap();

    // Oneshot must resolve (closed by cleanup dropping the sender)
    // within a tick — NOT wait the full 30s default tool timeout.
    let awaited = tokio::time::timeout(Duration::from_millis(500), result_rx).await;
    assert!(
        awaited.is_ok(),
        "forwarder failed to resolve oneshot within 500ms — pending leak?"
    );
    let recv_result = awaited.unwrap();
    assert!(
        recv_result.is_err(),
        "sender should have been dropped, leading to Err on the receiver"
    );

    // Pending table for this client is empty again.
    {
        let reg = registry.lock().unwrap();
        let state = reg.get(&client_id).expect("client state");
        assert!(
            state.pending_results.is_empty(),
            "pending_results leaked after failed broadcast: {:?}",
            state.pending_results.keys().collect::<Vec<_>>()
        );
    }

    drop(req_tx);
    let _ = forwarder.await;
}

/// N3 — When a client unregisters a tool (or disconnects) while a
/// ToolInvocation is already buffered in the mpsc but not yet
/// consumed by the forwarder, the request must resolve promptly
/// with an error carrying the shutdown reason — not sit there
/// waiting out the 30-second UdsTool timeout.
#[tokio::test]
async fn forwarder_drains_buffered_requests_on_shutdown() {
    use crate::domain::extension_tool::ToolInvocation;
    use std::time::Duration;

    let registry = new_client_tool_registry();
    let client_id: u64 = 101;
    {
        let mut reg = registry.lock().unwrap();
        reg.insert(client_id, ClientToolState::default());
    }

    let (writer_tx, _writer_rx) = tokio::sync::mpsc::channel::<String>(8);
    let (req_tx, req_rx) = tokio::sync::mpsc::channel::<ToolInvocation>(8);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<&'static str>();

    // Queue the requests AND fire shutdown BEFORE spawning the
    // forwarder. When the task starts, `biased; shutdown` wins
    // deterministically and the subsequent drain pass picks up
    // every already-buffered request.
    let (r1_tx, r1_rx) = tokio::sync::oneshot::channel();
    let (r2_tx, r2_rx) = tokio::sync::oneshot::channel();
    req_tx
        .send(ToolInvocation {
            tool_call_id: "drain-1".into(),
            tool_name: "weather".into(),
            arguments: "{}".into(),
            reply: r1_tx,
        })
        .await
        .unwrap();
    req_tx
        .send(ToolInvocation {
            tool_call_id: "drain-2".into(),
            tool_name: "weather".into(),
            arguments: "{}".into(),
            reply: r2_tx,
        })
        .await
        .unwrap();
    shutdown_tx.send("Tool unregistered").unwrap();

    // Drop our sender so the forwarder's drain loop eventually
    // sees an empty channel. (It already exits after the shutdown
    // branch fires, but closing the channel is defensive.)
    drop(req_tx);

    let forwarder = tokio::spawn(super::forward_tool_requests(
        client_id,
        "weather".to_string(),
        req_rx,
        shutdown_rx,
        registry.clone(),
        Some(writer_tx),
    ));

    let r1 = tokio::time::timeout(Duration::from_millis(500), r1_rx)
        .await
        .expect("buffered request 1 hung past 500ms")
        .expect("result_tx dropped without send");
    let r2 = tokio::time::timeout(Duration::from_millis(500), r2_rx)
        .await
        .expect("buffered request 2 hung past 500ms")
        .expect("result_tx dropped without send");

    assert!(r1.is_error);
    assert!(r1.content.contains("unregistered"));
    assert!(r2.is_error);
    assert!(r2.content.contains("unregistered"));

    let _ = forwarder.await;
}

/// Control case: when the targeted writer channel DOES have a
/// live receiver the forwarder leaves the pending entry in place
/// (the reader task will clear it when `tool_result` comes back).
#[tokio::test]
async fn forwarder_leaves_pending_when_writer_delivered() {
    use crate::domain::extension_tool::ToolInvocation;
    use std::time::Duration;

    let registry = new_client_tool_registry();
    let client_id: u64 = 7;
    {
        let mut reg = registry.lock().unwrap();
        reg.insert(client_id, ClientToolState::default());
    }

    let (writer_tx, mut writer_rx) = tokio::sync::mpsc::channel::<String>(8);

    let (req_tx, req_rx) = tokio::sync::mpsc::channel::<ToolInvocation>(4);
    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<&'static str>();
    let forwarder = tokio::spawn(super::forward_tool_requests(
        client_id,
        "weather".to_string(),
        req_rx,
        shutdown_rx,
        registry.clone(),
        Some(writer_tx),
    ));

    let (reply_tx, _result_rx) = tokio::sync::oneshot::channel();
    req_tx
        .send(ToolInvocation {
            tool_call_id: "uds-test-2".into(),
            tool_name: "weather".into(),
            arguments: "{}".into(),
            reply: reply_tx,
        })
        .await
        .unwrap();

    // Receive the targeted event (proving delivery happened).
    let line = tokio::time::timeout(Duration::from_millis(500), writer_rx.recv())
        .await
        .expect("writer event never arrived")
        .expect("writer channel closed unexpectedly");
    assert!(line.contains("\"execute_tool\""));

    // Short yield so the forwarder-side insert completes even if
    // the send path beat it to the broadcast receiver.
    tokio::time::sleep(Duration::from_millis(50)).await;

    {
        let reg = registry.lock().unwrap();
        let state = reg.get(&client_id).expect("client state");
        assert_eq!(
            state.pending_results.len(),
            1,
            "pending entry should remain until tool_result arrives"
        );
    }

    drop(req_tx);
    let _ = forwarder.await;
}

// --- #876: reader→ack wiring (ack_accepted_control) ----------------------------

#[tokio::test]
async fn ack_accepted_control_acks_via_writer_and_returns_follow_up_forward() {
    // The acceptance ack for a flagged prompt is delivered to THIS client's
    // serialized writer channel (bypassing the dispatch loop, so it works even
    // while a turn is in flight), id-correlated; the work becomes a queued
    // follow_up for the caller to enqueue.
    let reg = new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    register_client_writer(&reg, 7, tx);

    let ctrl = crate::interface::cli::uds_control_forward::intercept_control_forward(
        r#"{"type":"prompt","message":"do it","ack":"accept","id":"req-1"}"#,
    )
    .expect("flagged prompt should intercept");
    let forward = ack_accepted_control(&reg, 7, ctrl).await;

    let ack = rx
        .try_recv()
        .expect("ack written to the client's writer channel");
    let ackv: serde_json::Value = serde_json::from_str(ack.trim()).unwrap();
    assert_eq!(ackv["type"], "response");
    assert_eq!(ackv["id"], "req-1", "ack must echo the request id (#835)");

    let fwd: serde_json::Value =
        serde_json::from_str(&forward.expect("prompt forwards work")).unwrap();
    assert_eq!(fwd["type"], "follow_up");
    assert_eq!(fwd["message"], "do it");
}

#[tokio::test]
async fn ack_accepted_control_abort_acks_with_no_forward() {
    let reg = new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    register_client_writer(&reg, 1, tx);

    let ctrl = crate::interface::cli::uds_control_forward::intercept_control_forward(
        r#"{"type":"abort","ack":"accept","id":"a1"}"#,
    )
    .expect("flagged abort should intercept");
    let forward = ack_accepted_control(&reg, 1, ctrl).await;

    assert!(rx.try_recv().is_ok(), "abort still acks acceptance");
    assert!(
        forward.is_none(),
        "abort enqueues nothing (cancel already fired)"
    );
}
