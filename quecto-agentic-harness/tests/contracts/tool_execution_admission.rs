use quecto::domain::tool::ToolExecutionAdmission;
#[tokio::test]
async fn terminal_tools_are_explicitly_allowlisted_and_pause_denies_every_tool() {
    let (_directory, context) = super::swarm_control_fixture::context();
    context.check("bash", "{}").await.unwrap();
    context.pause("approval").unwrap();
    assert!(context.check("swarm", r#"{"op":"summary"}"#).await.is_err());
    context.resume_external().unwrap();
    context.cancel_run().unwrap();
    for op in ["summary", "events", "usage"] {
        context
            .check("swarm", &serde_json::json!({"op":op}).to_string())
            .await
            .unwrap();
    }
    for (name, arguments) in [
        ("bash", "{}"),
        ("swarm", r#"{"op":"run"}"#),
        ("swarm", "invalid"),
    ] {
        assert!(context.check(name, arguments).await.is_err());
    }
}
