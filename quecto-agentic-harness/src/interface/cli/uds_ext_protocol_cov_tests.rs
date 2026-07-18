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
async fn client_writer_tx_returns_clone_that_delivers_to_registered_writer() {
    let registry = new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);
    register_client_writer(&registry, 77, tx);

    let cloned = client_writer_tx(&registry, 77).expect("registered writer");
    cloned.send("hello extension".to_string()).await.unwrap();

    assert_eq!(rx.recv().await.as_deref(), Some("hello extension"));
    assert!(client_writer_tx(&registry, 88).is_none());
}
