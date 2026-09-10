//! Reporting-only admission for coordinators of ended (#1729) and terminal runs.
use super::*;

#[tokio::test]
async fn terminal_reports_are_admitted_only_for_the_retained_coordinator() {
    use crate::domain::provider::RequestAdmission;
    for status in ["failed", "blocked", "cancelled"] {
        let directory = tempfile::tempdir().unwrap();
        let parent = context(&directory);
        create(&parent, 2);
        parent
            .call("stop", json!([status, "retain diagnostic report"]))
            .unwrap();
        assert!(
            parent.check().await.is_ok(),
            "coordinator report unavailable for {status}"
        );
        let worker = SwarmContext {
            member: "worker".into(),
            ..parent.clone()
        };
        assert!(
            worker.check().await.is_err(),
            "terminal worker admitted for {status}"
        );
    }
}

#[tokio::test]
async fn terminal_tool_admission_allows_only_native_read_operations() {
    use crate::domain::tool::ToolExecutionAdmission;
    let directory = tempfile::tempdir().unwrap();
    let parent = context(&directory);
    create(&parent, 2);
    parent.call("stop", json!(["failed", "report"])).unwrap();
    for op in ["summary", "events", "usage"] {
        assert!(
            parent
                .check("swarm", &json!({"op":op}).to_string())
                .await
                .is_ok()
        );
    }
    for (name, args) in [
        ("bash", "{}"),
        ("spawn_agent", "{}"),
        ("swarm", r#"{"op":"run","code":"print(1)"}"#),
        ("swarm", r#"{"op":"resume"}"#),
        ("swarm", "{}"),
        ("swarm", "invalid"),
    ] {
        assert!(
            parent.check(name, args).await.is_err(),
            "admitted {name}: {args}"
        );
    }
    let worker = SwarmContext {
        member: "worker".into(),
        ..parent
    };
    assert!(worker.check("swarm", r#"{"op":"summary"}"#).await.is_err());
}

/// #1729: a run the coordinator ended keeps its execution registry open, so
/// the supervisor's resume lets the coordinator run Python again.
#[tokio::test]
async fn a_resumed_coordinator_runs_python_again_after_ending_the_run() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Arc::new(directory.path().to_path_buf());
    let tool = super::super::swarm_test_support::tool(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    );
    let context = SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
    };
    let ended = tool
        .execute(r#"{"code":"from swarm import board; board.stop('blocked','needs the master')"}"#)
        .await
        .unwrap();
    assert!(!ended.is_error, "{}", ended.content);
    let refused = tool
        .execute(r#"{"code":"print('while ended')"}"#)
        .await
        .unwrap();
    assert!(refused.is_error, "{}", refused.content);
    context.resume_external().unwrap();
    let again = tool
        .execute(r#"{"code":"from swarm import board; print(board.summary()['status'])"}"#)
        .await
        .unwrap();
    assert!(
        !again.is_error,
        "the registry closed on the end: {}",
        again.content
    );
    assert!(again.content.contains("running"), "{}", again.content);
}
