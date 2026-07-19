use super::*;

fn tool_reg(name: &str) -> ToolRegistration {
    ToolRegistration {
        name: name.to_string(),
        description: format!("{name} description"),
        parameters_schema: r#"{"type":"object"}"#.to_string(),
    }
}

#[test]
fn unregister_response_lists_only_removed_registered_tools() {
    let registry = new_client_tool_registry();
    let core = HashSet::new();
    let (ok, _, _) = handle_register_tools(RegisterToolsArgs {
        client_id: 9,
        id: None,
        tools: &[tool_reg("alpha"), tool_reg("beta")],
        registry: &registry,
        core_tool_names: &core,
    });
    assert!(ok);

    let (event, removed) = handle_unregister_tools(
        9,
        Some("unreg-1"),
        &["alpha".to_string(), "missing".to_string()],
        &registry,
    );

    assert_eq!(removed, vec!["alpha"]);
    let json = event.to_json_line();
    assert!(json.contains("\"id\":\"unreg-1\""), "json: {json}");
    assert!(
        json.contains("\"unregistered\":[\"alpha\"]"),
        "json: {json}"
    );
    let locked = registry.lock().unwrap();
    assert!(!locked[&9].tool_names.contains("alpha"));
    assert!(locked[&9].tool_names.contains("beta"));
}

#[test]
fn register_tools_rejects_core_shadow_duplicate_and_other_client_collision() {
    let registry = new_client_tool_registry();
    let core = HashSet::from(["bash".to_string()]);

    let (ok, ev, tools) = handle_register_tools(RegisterToolsArgs {
        client_id: 1,
        id: Some("shadow"),
        tools: &[tool_reg("bash")],
        registry: &registry,
        core_tool_names: &core,
    });
    assert!(!ok);
    assert!(tools.is_empty());
    assert!(ev.to_json_line().contains("shadows a core tool"));

    let (ok, ev, _) = handle_register_tools(RegisterToolsArgs {
        client_id: 1,
        id: Some("dup"),
        tools: &[tool_reg("ext"), tool_reg("ext")],
        registry: &registry,
        core_tool_names: &HashSet::new(),
    });
    assert!(!ok);
    assert!(ev.to_json_line().contains("registered more than once"));

    assert!(
        handle_register_tools(RegisterToolsArgs {
            client_id: 1,
            id: None,
            tools: &[tool_reg("owned")],
            registry: &registry,
            core_tool_names: &HashSet::new(),
        })
        .0
    );
    let (ok, ev, _) = handle_register_tools(RegisterToolsArgs {
        client_id: 2,
        id: Some("collision"),
        tools: &[tool_reg("owned")],
        registry: &registry,
        core_tool_names: &HashSet::new(),
    });
    assert!(!ok);
    assert!(ev.to_json_line().contains("already registered by client 1"));
}

#[tokio::test]
async fn tool_result_for_pending_sender_delivers_and_sweeps_expired_entries() {
    let registry = new_client_tool_registry();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (expired_tx, expired_rx) = tokio::sync::oneshot::channel();
    {
        let mut reg = registry.lock().unwrap();
        let state = reg.entry(5).or_default();
        state.insert_pending("live".to_string(), tx, std::time::Duration::from_secs(30));
        state.pending_results.insert(
            "expired".to_string(),
            PendingResult {
                reply: expired_tx,
                deadline: std::time::Instant::now() - std::time::Duration::from_millis(1),
            },
        );
    }

    handle_tool_result(ToolResultArgs {
        client_id: 5,
        tool_call_id: "live",
        content: "done",
        is_error: false,
        registry: &registry,
    });

    let result = rx.await.expect("live result delivered");
    assert_eq!(result.content, "done");
    assert!(!result.is_error);
    assert!(expired_rx.await.is_err(), "expired sender was dropped");
    assert!(registry.lock().unwrap()[&5].pending_results.is_empty());
}

#[test]
fn tool_result_for_unknown_client_does_not_panic_or_create_state() {
    let registry = new_client_tool_registry();

    handle_tool_result(ToolResultArgs {
        client_id: 123,
        tool_call_id: "missing-call",
        content: "late result",
        is_error: false,
        registry: &registry,
    });

    assert!(registry.lock().unwrap().is_empty());
}

#[tokio::test]
async fn client_writer_tx_returns_clone_and_ack_accepted_control_forwards_optional_work() {
    let registry = new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(2);
    register_client_writer(&registry, 77, tx);

    let cloned = client_writer_tx(&registry, 77).expect("registered writer");
    cloned.send("hello extension".to_string()).await.unwrap();
    assert_eq!(rx.recv().await.as_deref(), Some("hello extension"));
    assert!(client_writer_tx(&registry, 88).is_none());

    let forward = ack_accepted_control(
        &registry,
        77,
        super::super::uds_control_forward::AcceptedControl {
            ack_line: "ack\n".to_string(),
            forward_line: Some("work".to_string()),
        },
    )
    .await;
    assert_eq!(forward.as_deref(), Some("work"));
    assert_eq!(rx.recv().await.as_deref(), Some("ack\n"));

    let forward = ack_accepted_control(
        &registry,
        88,
        super::super::uds_control_forward::AcceptedControl {
            ack_line: "lost\n".to_string(),
            forward_line: None,
        },
    )
    .await;
    assert!(forward.is_none());
}

#[test]
fn handle_client_disconnect_removes_state_and_errors_pending_execution() {
    let registry = new_client_tool_registry();
    let (tx, rx) = tokio::sync::oneshot::channel();
    assert!(
        handle_register_tools(RegisterToolsArgs {
            client_id: 44,
            id: None,
            tools: &[tool_reg("bye")],
            registry: &registry,
            core_tool_names: &HashSet::new(),
        })
        .0
    );
    registry
        .lock()
        .unwrap()
        .get_mut(&44)
        .unwrap()
        .insert_pending("call".to_string(), tx, std::time::Duration::from_secs(30));

    let removed = handle_client_disconnect(44, &registry);

    assert_eq!(removed, vec!["bye".to_string()]);
    assert!(!registry.lock().unwrap().contains_key(&44));
    let result = rx.blocking_recv().expect("disconnect result");
    assert_eq!(result.content, "Extension disconnected");
    assert!(result.is_error);
    assert!(handle_client_disconnect(44, &registry).is_empty());
}
