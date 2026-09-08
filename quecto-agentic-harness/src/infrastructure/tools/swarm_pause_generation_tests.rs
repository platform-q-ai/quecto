use super::*;
use crate::domain::tool::Tool;
#[tokio::test]
async fn stale_pause_settlement_preserves_resumed_python_job() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = std::sync::Arc::new(directory.path().to_path_buf());
    let tool = super::super::swarm_test_support::tool(
        workspace.clone(),
        std::sync::Arc::new(crate::infrastructure::security::sandbox::Sandbox::new(
            Some(workspace.as_ref().clone()),
        )),
        super::super::swarm::SwarmConfig::default(),
    );
    let context = SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    context.pause("old pause").unwrap();
    let old = context.snapshot().unwrap();
    context.resume().unwrap();
    let started = tool.execute(r#"{"op":"run","code":"import time; time.sleep(0.1); print('resumed')","background":true}"#).await.unwrap();
    let started: Value = serde_json::from_str(&started.content).unwrap();
    crate::application::swarm::settle(&old, &context.member, &RuntimeProcesses(&context))
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let result = tool
            .execute(&json!({"op":"status","job_id":started["job_id"]}).to_string())
            .await
            .unwrap();
        let value: Value = serde_json::from_str(&result.content).unwrap();
        if matches!(value["status"].as_str(), Some("running" | "cancelling")) {
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        } else {
            assert_eq!(value["status"], "completed", "{value}");
            break;
        }
    }
}
