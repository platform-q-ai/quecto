use super::*;
use crate::domain::swarm::CoordinationPort;
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

/// #1715: creating a run makes the creator a coordinator, which a workflow
/// engine cannot be; a workflow-free creator succeeds and becomes a participant.
#[tokio::test]
async fn a_workflow_enabled_agent_cannot_create_a_swarm() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let context = SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    let identity = crate::domain::swarm::ProcessIdentity {
        pid: std::process::id(),
        started: super::super::swarm_bridge::process_start(std::process::id()).unwrap(),
    };
    context.join(&identity, None, None).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let input = json!({"goal":"g","constraints":[],
        "criteria":[{"id":"t","kind":"command","description":"pass"}],
        "member_limit":2,"deadline":deadline});
    let participation = super::super::swarm_bridge::Participation::shared();
    // An engaged workflow: guards on.
    let engaged: super::super::swarm_bridge::WorkflowEngineSlot = Default::default();
    let _ = engaged.set(std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            true,
        )
        .unwrap(),
    )));
    let error = control_with_workflow(
        context.clone(),
        "create",
        input.clone(),
        participation.clone(),
        engaged,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("cannot create a swarm"), "{error}");
    assert_eq!(
        context.snapshot().unwrap().status,
        crate::domain::swarm::RunStatus::Setup,
        "nothing was created"
    );
    assert!(!participation.participating(), "nothing was created");
    let created = control_with_workflow(
        context.clone(),
        "create",
        input,
        participation.clone(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(created["status"], "running");
    assert_eq!(
        context.snapshot().unwrap().status,
        crate::domain::swarm::RunStatus::Running
    );
    assert!(
        participation.participating(),
        "the creator is now a swarm agent"
    );
    context.cancel_run().unwrap();
}
