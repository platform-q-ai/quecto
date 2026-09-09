use super::*;
use serde_json::json;
#[tokio::test]
async fn native_supervisor_validates_budget_and_event_requests_without_mutation() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    for (op, input) in [
        ("usage_budget", json!({})),
        (
            "usage_budget",
            json!({"token_limit":4,"strict_unknown":"yes"}),
        ),
        ("usage_budget", json!({"token_limit":-1})),
        ("events", json!({"limit":0})),
        ("events", json!({"after":"bad"})),
        ("unknown", json!({})),
    ] {
        assert!(control(context.clone(), op, input).await.is_err());
    }
    let configured = control(context.clone(), "usage_budget", json!({"token_limit":100}))
        .await
        .unwrap();
    assert_eq!(configured["budget"]["token_limit"], 100);
    assert_eq!(
        control(context.clone(), "usage", json!({})).await.unwrap()["budget"]["strict_unknown"],
        true
    );
    assert!(
        control(context.clone(), "usage_budget", json!({"token_limit":null}))
            .await
            .unwrap()["budget"]["token_limit"]
            .is_null()
    );
    let paused = control(context.clone(), "pause", json!({"reason":"approval"}))
        .await
        .unwrap();
    assert_eq!(paused["status"], "paused");
    let resumed = control(context.clone(), "resume", json!({})).await.unwrap();
    assert_eq!(resumed["status"], "running");
    let cancelled = control(context, "cancel_run", json!({})).await.unwrap();
    assert_eq!(cancelled["status"], "cancelled");
}

fn public_tool(context: SwarmContext) -> super::super::swarm::SwarmTool {
    use std::sync::Arc;
    let checkout = context.checkout.clone();
    super::super::swarm::SwarmTool::new(
        Arc::new(checkout.clone()),
        Arc::new(crate::infrastructure::security::sandbox::Sandbox::new(
            Some(checkout),
        )),
        Default::default(),
    )
    .with_context(Some(context))
}

#[tokio::test]
async fn public_swarm_tool_reports_usage_after_completion() {
    use crate::domain::tool::Tool;
    let (_directory, context) = crate::swarm_control_fixture::context();
    context.cancel_run().unwrap();
    let result = public_tool(context)
        .execute(r#"{"op":"usage"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let report: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert!(report.get("budget").is_some(), "{report}");
}

#[tokio::test]
async fn public_swarm_tool_configures_and_disables_usage_budget() {
    use crate::domain::tool::Tool;
    let (_directory, context) = crate::swarm_control_fixture::context();
    let tool = public_tool(context.clone());
    for limit in [json!(100), serde_json::Value::Null] {
        let result = tool
            .execute(&json!({"op":"usage_budget","token_limit":limit}).to_string())
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(
            context.usage_report().unwrap()["budget"]["token_limit"],
            limit
        );
    }
}
