use super::*;
use crate::application::swarm::ports::CoordinationPort;
use serde_json::json;
#[tokio::test]
async fn native_supervisor_validates_event_requests_without_mutation() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    // Owner decision, 2026-10-14: no token budget limits a swarm, so
    // `usage_budget` is an unknown op, whatever its fields.
    for (op, input) in [
        ("usage_budget", json!({"token_limit":100})),
        ("usage_budget", json!({"token_limit":null})),
        ("events", json!({"limit":0})),
        ("events", json!({"after":"bad"})),
        ("unknown", json!({})),
    ] {
        assert!(control(context.clone(), op, input).await.is_err(), "{op}");
    }
    let refused = control(context.clone(), "usage_budget", json!({"token_limit":100}))
        .await
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "tool error: unknown swarm operation usage_budget"
    );
    assert_eq!(
        control(context.clone(), "usage", json!({})).await.unwrap()["budget"]["token_limit"],
        serde_json::Value::Null,
        "no budget was written"
    );
    let paused = control(context.clone(), "pause", json!({"reason":"approval"}))
        .await
        .unwrap();
    assert_eq!(paused["status"], "paused");
    let refused = control(context.clone(), "resume", json!({}))
        .await
        .expect_err("a member cannot resume the run");
    assert!(
        refused.to_string().contains("outside the swarm"),
        "{refused}"
    );
    let resumed =
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.resume_external())
            .unwrap();
    assert_eq!(resumed["status"], "running");
    let cancelled = control(context, "cancel_run", json!({})).await.unwrap();
    assert_eq!(cancelled["status"], "cancelled");
}

fn public_tool(context: SwarmContext) -> super::super::swarm::SwarmTool {
    super::super::swarm::SwarmTool::new().with_context(Some(context))
}

#[tokio::test]
async fn public_swarm_tool_reports_usage_after_completion() {
    use crate::application::tools::ports::Tool;
    let (_directory, context) = crate::swarm_control_fixture::context();
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.cancel_run()).unwrap();
    let result = public_tool(context)
        .execute(r#"{"op":"usage"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let report: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert!(report.get("budget").is_some(), "{report}");
}

/// The public `swarm` tool takes no `usage_budget` op: it is refused, and
/// the schema it advertises names neither the op nor its fields.
#[tokio::test]
async fn public_swarm_tool_takes_no_usage_budget_op() {
    use crate::application::tools::ports::Tool;
    let (_directory, context) = crate::swarm_control_fixture::context();
    let tool = public_tool(context.clone());
    let result = tool
        .execute(&json!({"op":"usage_budget","token_limit":100}).to_string())
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.content);
    let schema = tool.definition().parameters_schema;
    for gone in ["usage_budget", "token_limit", "strict_unknown"] {
        assert!(!schema.contains(gone), "{gone}");
    }
    assert!(
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.usage_report())
            .unwrap()["budget"]["token_limit"]
            .is_null()
    );
}

/// #1715: creating a run makes the creator a coordinator, which a workflow
/// engine cannot be; a workflow-free creator succeeds and becomes a participant.
#[tokio::test]
async fn a_workflow_enabled_agent_cannot_create_a_swarm() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let context = SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    let identity = crate::domain::swarm::ProcessIdentity {
        pid: std::process::id(),
        started: super::super::swarm_bridge::process_start(std::process::id()).unwrap(),
    };
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.join(&identity, None, None)
    })
    .unwrap();
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
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.snapshot())
            .unwrap()
            .status,
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
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.snapshot())
            .unwrap()
            .status,
        crate::domain::swarm::RunStatus::Running
    );
    assert!(
        participation.participating(),
        "the creator is now a swarm agent"
    );
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.cancel_run()).unwrap();
}
