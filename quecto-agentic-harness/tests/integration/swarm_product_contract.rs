//! Product regressions, also runnable as a compiled test binary in a fresh container.
use quecto::application::tools::ports::Tool;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::swarm::SwarmTool;
use serde_json::{Value, json};
use std::sync::Arc;

fn fixture() -> (tempfile::TempDir, Arc<std::path::PathBuf>, SwarmTool) {
    let root = tempfile::tempdir().unwrap();
    let workspace = Arc::new(
        root.path()
            .join(".quecto/container-environments/environment/workspace/repo"),
    );
    std::fs::create_dir_all(workspace.as_ref()).unwrap();
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
        lifecycle: Arc::new(quecto::application::swarm::LifecycleService),
    };
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    quecto::infrastructure::tools::call_work::off_the_runtime(|| context.create_run(&json!({"goal":"product contract","constraints":[],"criteria":[{"id":"tests","kind":"command","description":"pass"}],"member_limit":1,"deadline":deadline}),
        &quecto::domain::swarm::ProcessIdentity { pid: std::process::id(), started: quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap() }, None)).unwrap();
    let tool = SwarmTool::new().with_context(Some(context));

    (root, workspace, tool)
}
async fn execute(tool: &SwarmTool, input: Value) -> Value {
    let result = tool.execute(&input.to_string()).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    serde_json::from_str(&result.content).unwrap()
}
#[tokio::test]
async fn completed_run_keeps_summary_and_normal_artifact_export_serviceable() {
    let (_root, workspace, tool) = fixture();
    std::fs::write(workspace.join("report.txt"), "final-evidence").unwrap();
    execute(
        &tool,
        json!({"op":"evidence","criterion":"tests","artifact":"report.txt","revision":"R1",
            "kind":"command","passed":true}),
    )
    .await;
    execute(&tool, json!({"op":"complete","revision":"R1"})).await;
    let summary = execute(&tool, json!({"op":"summary"})).await;
    assert_eq!(
        (summary["status"].as_str(), summary["outcome"].as_str()),
        (Some("paused"), Some("succeeded"))
    );
    let rejected = tool.execute(r#"{"op":"inbox"}"#).await.unwrap();
    assert!(rejected.is_error, "{}", rejected.content);
    let bash = quecto::infrastructure::tools::bash::ExecTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
    );
    let exported = bash
        .execute(r#"{"command":"cat report.txt"}"#)
        .await
        .unwrap();
    assert!(!exported.is_error, "{}", exported.content);
    assert!(exported.content.contains("final-evidence"));
}

#[test]
fn isolated_context_is_detected_when_container_contract_is_supplied() {
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext::discover(
        Arc::new(quecto::application::swarm::LifecycleService),
        quecto::composition::swarm::swarm_board(),
    );
    if std::env::var("QUECTO_SWARM_CONTAINER").as_deref() == Ok("isolated-pid-v1") {
        assert!(
            context.is_some(),
            "container runtime contract was not recognized"
        );
    } else {
        assert!(context.is_none());
    }
}
