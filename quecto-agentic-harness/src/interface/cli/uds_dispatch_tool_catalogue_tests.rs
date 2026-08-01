use super::cov_tests::{Fixture, tool_reg};
use super::dispatch_command;
use crate::interface::cli::uds::AgentCommand;

#[tokio::test]
async fn dispatch_unregister_tools_emits_one_catalogue_change() {
    let mut fx = Fixture::new();
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);

    let reg_cmd = AgentCommand::RegisterTools {
        id: Some("rt".into()),
        tools: vec![tool_reg("weather"), tool_reg("clock")],
    };
    {
        let mut ctx = fx.ctx();
        ctx.broadcast_tx = Some(tx.clone());
        ctx.stdout = None;
        assert!(!dispatch_command(reg_cmd, &mut ctx).await);
    }
    while rx.try_recv().is_ok() {}

    let unreg_cmd = AgentCommand::UnregisterTools {
        id: Some("ut".into()),
        tools: vec!["weather".into(), "clock".into()],
    };
    {
        let mut ctx = fx.ctx();
        ctx.broadcast_tx = Some(tx);
        ctx.stdout = None;
        assert!(!dispatch_command(unreg_cmd, &mut ctx).await);
    }

    let mut catalogue_events = Vec::new();
    while let Ok(line) = rx.try_recv() {
        let event: serde_json::Value = serde_json::from_str(line.trim()).expect("event json");
        if event.get("type").and_then(|value| value.as_str()) == Some("tool_catalogue_changed") {
            catalogue_events.push(event);
        }
    }

    assert_eq!(catalogue_events.len(), 1, "events: {catalogue_events:#?}");
    let event = &catalogue_events[0];
    assert_eq!(event["reason"], "unregister_tool");
    assert_eq!(
        event["changedTools"],
        serde_json::json!(["weather", "clock"])
    );
    assert!(
        event["before"].to_string().contains("weather")
            && event["before"].to_string().contains("clock"),
        "before should include both removed tools: {event:#?}"
    );
    assert!(
        !event["after"].to_string().contains("weather")
            && !event["after"].to_string().contains("clock"),
        "after should exclude both removed tools: {event:#?}"
    );
}
