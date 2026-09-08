//! Product regressions, also runnable as a compiled test binary in a fresh container.
use quecto::domain::tool::Tool;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::swarm::{SwarmConfig, SwarmTool};
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
    context.create_run(&json!({"goal":"product contract","constraints":[],"criteria":[{"id":"tests","kind":"command","description":"pass"}],"member_limit":1,"deadline":deadline}),
        &quecto::domain::swarm::ProcessIdentity { pid: std::process::id(), started: quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap() }, None).unwrap();
    let tool = SwarmTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    )
    .with_context(Some(context));

    (root, workspace, tool)
}
async fn execute(tool: &SwarmTool, input: Value) -> Value {
    let result = tool.execute(&input.to_string()).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    serde_json::from_str(&result.content).unwrap()
}
fn resolvable(value: &Value, workspace: &std::path::Path) {
    assert_eq!(value["artifact_namespace"], "workspace-relative");
    assert_eq!(value["artifact_base"], workspace.to_string_lossy().as_ref());
    for path in value["artifact_paths"].as_array().unwrap() {
        let path = path.as_str().unwrap();
        assert!(
            path.starts_with(".quecto/swarm/"),
            "wrong relative path: {path}"
        );
        assert!(
            workspace.join(path).is_file(),
            "unreadable artifact: {path}"
        );
    }
}
#[tokio::test]
async fn nested_workspace_spill_and_background_paths_share_one_namespace() {
    let (_root, workspace, tool) = fixture();
    let spill = execute(
        &tool,
        json!({"op":"run","code":"print('x'*20000)","max_output_bytes":100}),
    )
    .await;
    resolvable(&spill, &workspace);
    let job = execute(&tool, json!({"op":"run","code":"print('ready',flush=True); import time; time.sleep(30)","background":true})).await;
    let id = job["job_id"].clone();
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let value = execute(&tool, json!({"op":"output","job_id":id})).await;
            if value["stdout"].as_str().unwrap().contains("ready")
                && value["artifact_paths"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|p| workspace.join(p.as_str().unwrap()).is_file())
            {
                break value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let status = execute(&tool, json!({"op":"status","job_id":id})).await;
    resolvable(&output, &workspace);
    resolvable(&status, &workspace);
    assert_eq!(status["artifact_paths"], output["artifact_paths"]);
    execute(&tool, json!({"op":"cancel","job_id":id})).await;
}
#[tokio::test]
async fn process_limit_errors_explain_supported_command_routing() {
    let (_root, _workspace, tool) = fixture();
    // Deterministic across privileged test runners that do not enforce NPROC.
    let result = tool.execute(&json!({"op":"run","code":"raise BlockingIOError(11, 'Resource temporarily unavailable')"}).to_string()).await.unwrap();
    assert!(result.is_error);
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert!(value["diagnostic"].as_str().unwrap_or("").contains("bash"));
    assert_eq!(value["resource_limits"]["processes"], 1);
}

#[tokio::test]
async fn default_python_restricts_subprocesses_but_bash_routes_commands() {
    // RLIMIT_NPROC does not constrain privileged root runners. The deterministic
    // diagnostic regression above still covers their result contract.
    // SAFETY: geteuid takes no arguments and only reads the effective user ID.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let (_root, workspace, tool) = fixture();
    let result = tool.execute(&json!({"op":"run","code":"import resource,subprocess; print(resource.getrlimit(resource.RLIMIT_NPROC)); subprocess.run(['sh','-c','printf unexpected'],check=True)"}).to_string()).await.unwrap();
    assert!(
        result.is_error,
        "default non-root Python unexpectedly spawned a child: {}",
        result.content
    );
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert!(value["stdout"].as_str().unwrap().contains("(1, 1)"));
    assert!(value["diagnostic"].as_str().unwrap().contains("bash"));
    let bash = quecto::infrastructure::tools::bash::ExecTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
    );
    let result = bash
        .execute(r#"{"command":"printf 'needle\nhay\n' | grep needle"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("needle"));
}

#[tokio::test]
async fn completed_run_keeps_summary_and_normal_artifact_export_serviceable() {
    let (_root, workspace, tool) = fixture();
    std::fs::write(workspace.join("report.txt"), "final-evidence").unwrap();
    execute(&tool, json!({"op":"run","code":"from swarm import board; board.evidence('tests','report.txt','R1','command',True); board.complete('R1')"})).await;
    let summary = execute(&tool, json!({"op":"summary"})).await;
    assert_eq!(summary["status"], "succeeded");
    let rejected = tool
        .execute(r#"{"op":"run","code":"from swarm import board; board.inbox()"}"#)
        .await
        .unwrap();
    assert!(rejected.is_error);
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
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext::discover(Arc::new(
        quecto::application::swarm::LifecycleService,
    ));
    if std::env::var("QUECTO_SWARM_CONTAINER").as_deref() == Ok("isolated-pid-v1") {
        assert!(
            context.is_some(),
            "container runtime contract was not recognized"
        );
    } else {
        assert!(context.is_none());
    }
}
