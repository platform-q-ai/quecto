use super::swarm::{SwarmConfig, SwarmTool};
use super::swarm_bridge::{SwarmContext, process_confirmed_dead, process_start};
use crate::domain::tool::Tool;
use crate::infrastructure::security::sandbox::Sandbox;
use serde_json::json;
use std::sync::Arc;

fn context(directory: &tempfile::TempDir) -> SwarmContext {
    SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "parent".into(),
    }
}

fn create(context: &SwarmContext, limit: u64) {
    std::fs::create_dir_all(context.checkout.join(".quecto")).unwrap();
    context.call("create", json!(["ship", [], [{"id":"tests","kind":"command","description":"pass"}], limit,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 300])).unwrap();
}

#[test]
fn isolated_bootstrap_loads_compiled_helpers_not_checkout_modules() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("swarm.py"),
        "raise RuntimeError('poisoned')",
    )
    .unwrap();
    let context = context(&directory);
    create(&context, 1);
    assert_eq!(context.summary().unwrap()["goal"], "ship");
    assert_eq!(context.summary().unwrap()["usage"], 1);
}

#[test]
fn missing_and_corrupt_databases_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    assert!(
        context
            .summary()
            .unwrap_err()
            .to_string()
            .contains("coordination")
    );
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    std::fs::write(context.database(), "corrupt").unwrap();
    assert!(
        context
            .summary()
            .unwrap_err()
            .to_string()
            .contains("coordination")
    );
}

#[tokio::test]
async fn host_tool_rejects_execution_even_with_a_planted_store() {
    let directory = tempfile::tempdir().unwrap();
    create(&context(&directory), 1);
    let workspace = Arc::new(directory.path().to_path_buf());
    let tool = SwarmTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    );
    let result = tool
        .execute(r#"{"op":"run","code":"print('escaped')"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("container-only"));
}

#[test]
fn startup_admission_counts_members_before_run_creation() {
    let directory = tempfile::tempdir().unwrap();
    let parent = context(&directory);
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    parent
        .call("_bootstrap", json!([123, "start", "/parent", null]))
        .unwrap();
    let worker = SwarmContext {
        member: "worker".into(),
        ..parent.clone()
    };
    worker
        .call("_bootstrap", json!([124, "start", "/worker", null]))
        .unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let args = json!(["ship", [], [{"id":"t","kind":"command","description":"pass"}], 1, deadline]);
    assert!(
        parent
            .call("create", args.clone())
            .unwrap_err()
            .to_string()
            .contains("existing live")
    );
    assert!(
        worker
            .call("create", args)
            .unwrap_err()
            .to_string()
            .contains("coordinator")
    );
    assert_eq!(parent.summary().unwrap()["usage"], 2);
}

#[test]
fn failed_launch_releases_capacity_but_live_reservation_does_not() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 2);
    {
        let reservation =
            super::swarm_admission::LaunchReservation::reserve(context.clone()).unwrap();
        assert_eq!(context.summary().unwrap()["usage"], 2);
        assert!(super::swarm_admission::LaunchReservation::reserve(context.clone()).is_err());
        drop(reservation);
    }
    assert_eq!(context.summary().unwrap()["usage"], 1);
    let mut reservation =
        super::swarm_admission::LaunchReservation::reserve(context.clone()).unwrap();
    let mut command = tokio::process::Command::new("true");
    reservation.configure(&mut command);
    reservation.launched(std::process::id()).unwrap();
    drop(reservation);
    assert_eq!(
        super::swarm_lifecycle::reconcile(&context).unwrap()["usage"],
        2
    );
}

#[test]
fn only_confirmed_death_recovers_a_recorded_launch() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 2);
    context.call("_admit", json!(["worker", "r"])).unwrap();
    context
        .call(
            "_record_launch",
            json!(["worker", "r", std::process::id(), "recycled-start-time"]),
        )
        .unwrap();
    assert_eq!(
        super::swarm_lifecycle::reconcile(&context).unwrap()["usage"],
        1
    );
    assert!(!process_confirmed_dead(
        std::process::id(),
        &process_start(std::process::id()).unwrap()
    ));
    assert!(process_confirmed_dead(std::process::id(), "stale"));
}

#[test]
fn legacy_resource_configuration_is_preserved_without_parallel_tools() {
    let tools: crate::infrastructure::config::ToolsConfig = serde_json::from_value(json!({"python_lab":{"max_cpu_seconds":3,"max_memory_bytes":123456,"max_concurrent_jobs":1}})).unwrap();
    assert_eq!(tools.swarm.max_cpu_seconds, Some(3));
    assert_eq!(tools.swarm.max_memory_bytes, Some(123456));
    assert_eq!(tools.swarm.max_concurrent_jobs, 1);
    let serialized = serde_json::to_value(tools).unwrap();
    assert!(serialized.get("python_lab").is_none());
    assert!(serialized.get("swarm").is_some());
}

#[tokio::test]
async fn cancelling_run_stops_background_python_and_preserves_progress() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Arc::new(directory.path().to_path_buf());
    let tool = super::swarm_test_support::tool(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    );
    let started = tool
        .execute(r#"{"op":"run","code":"import time; time.sleep(30)","background":true}"#)
        .await
        .unwrap();
    assert!(!started.is_error, "{}", started.content);
    let job: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    let cancelled = tool.execute(r#"{"op":"cancel_run"}"#).await.unwrap();
    assert!(!cancelled.is_error, "{}", cancelled.content);
    assert!(cancelled.content.contains("cancelled"));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let result = tool
            .execute(&json!({"op":"status","job_id":job["job_id"]}).to_string())
            .await
            .unwrap();
        let status: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        if status["status"] == "cancelled" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "background work survived cancellation: {status}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let rejected = tool
        .execute(r#"{"op":"run","code":"print('new work')"}"#)
        .await
        .unwrap();
    assert!(rejected.is_error);
}

#[tokio::test]
async fn run_creation_requires_authorized_container_and_bounded_policy() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let workspace = Arc::new(directory.path().to_path_buf());
    let tool = SwarmTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    )
    .with_context(Some(context.clone()));
    let invalid = tool
        .execute(
            r#"{"op":"create","goal":"ship","constraints":[],"criteria":[],"member_limit":11}"#,
        )
        .await
        .unwrap();
    assert!(invalid.is_error);
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let created = tool.execute(&json!({"op":"create","goal":"ship","constraints":[],"criteria":[{"id":"test","kind":"command","description":"pass"}],"member_limit":1,"deadline":deadline}).to_string()).await.unwrap();
    assert!(!created.is_error, "{}", created.content);
    assert_eq!(context.summary().unwrap()["usage"], 1);
    assert!(
        context
            .call("_admit", json!(["worker", "r"]))
            .unwrap_err()
            .to_string()
            .contains("reuse")
    );
}
