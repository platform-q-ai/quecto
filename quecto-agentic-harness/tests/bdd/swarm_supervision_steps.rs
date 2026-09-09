use super::{QuectoWorld, result_json};
use cucumber::{then, when};
use serde_json::json;
#[path = "swarm_supervision_fixture.rs"]
mod fixture;

#[when("the supervisor pauses active work then delivers approval and exports evidence")]
fn supervise(world: &mut QuectoWorld) {
    let workspace = world.swarm_workspace.clone().unwrap();
    let evidence = std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(exercise(workspace))
    })
    .join()
    .unwrap();
    world.swarm_result = Some(quecto::domain::tool::ToolResult {
        content: evidence.to_string(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    });
}
async fn exercise(workspace: std::path::PathBuf) -> serde_json::Value {
    let runtime = fixture::Runtime::start(&workspace).await;
    runtime
        .command(json!({"type":"prompt","message":"Initialise the swarm","ack":"accept"}))
        .await;
    runtime.wait_report("READY").await;
    runtime
        .command(json!({"type":"prompt","message":"Wait for approval","ack":"accept"}))
        .await;
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while runtime.requests.load(std::sync::atomic::Ordering::SeqCst) < 3 {
        assert!(
            tokio::time::Instant::now() < until,
            "working request never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let paused = runtime
        .command(json!({"type":"swarm_control","action":"pause","reason":"approval"}))
        .await;
    runtime.wait_idle().await;
    let approval = runtime
        .command(json!({"type":"follow_up","message":"Approved schema v2","ack":"accept"}))
        .await;
    runtime.wait_receipt(&approval["id"], "queued").await;
    runtime
        .command(json!({"type":"swarm_control","action":"resume"}))
        .await;
    runtime
        .command(json!({"type":"prompt","message":"Continue approved work","ack":"accept"}))
        .await;
    runtime.wait_report("APPROVED").await;
    let receipt = runtime.wait_receipt(&approval["id"], "completed").await;
    let budget = runtime.command(json!({"type":"swarm_control","action":"usage_budget","token_limit":1,"strict_unknown":false})).await;
    runtime
        .command(json!({"type":"swarm_control","action":"usage_budget","token_limit":null}))
        .await;
    runtime
        .command(json!({"type":"swarm_control","action":"resume"}))
        .await;
    let report = runtime
        .command(json!({"type":"get_report","export_raw":true}))
        .await;
    let raw =
        std::fs::read_to_string(report["data"]["rawExport"]["path"].as_str().unwrap()).unwrap();
    let stats = runtime.command(json!({"type":"get_session_stats"})).await;
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        checkout: workspace.clone(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
    };
    context.cancel_run().unwrap();
    let terminal = runtime.command(json!({"type":"get_report"})).await;
    let evidence = json!({"paused":paused["data"]["status"],"approval":receipt["status"],"budget":budget["data"]["status"],"raw_has_approval":raw.contains("Approved schema v2"),"report":terminal["data"]["report"]["content"],"requests":stats["data"]["requestDiagnostics"]["logical_requests"],"usage":context.usage_report().unwrap()});
    runtime.finish().await;
    evidence
}

#[then("the swarm approval has a completed receipt and a retained terminal report")]
fn approved(world: &mut QuectoWorld) {
    let evidence = result_json(world);
    assert_eq!(evidence["paused"], "paused");
    assert_eq!(evidence["approval"], "completed");
    assert_eq!(evidence["report"], "APPROVED");
    assert_eq!(evidence["raw_has_approval"], true);
}
#[then("the observed usage budget pauses the run and request accounting is available")]
fn observed_budget(world: &mut QuectoWorld) {
    let evidence = result_json(world);
    assert_eq!(evidence["budget"], "paused");
    assert!(evidence["requests"].as_u64().unwrap() >= 4);
    assert!(
        evidence["usage"]["totals"]["observed_tokens"]
            .as_u64()
            .unwrap()
            > 0
    );
}

fn failing_then_recovering(index: usize) -> fixture::Reply {
    match index {
        1 => fixture::Reply::Text("READY"),
        2 => fixture::Reply::TerminalFailure,
        _ => fixture::Reply::Text("RECOVERED"),
    }
}

/// #1721: a member suspended by a terminal provider failure is brought back
/// by the parent's `swarm_control resume` alone; no prompt or steer.
#[when("a provider failure suspends the coordinator and the parent resumes the run")]
fn resume_restores(world: &mut QuectoWorld) {
    let workspace = world.swarm_workspace.clone().unwrap();
    let evidence = std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(exercise_resume(workspace))
    })
    .join()
    .unwrap_or_else(|payload| {
        let text = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "opaque panic".into());
        panic!("resume exercise failed: {text}")
    });
    world.swarm_result = Some(quecto::domain::tool::ToolResult {
        content: evidence.to_string(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    });
}

async fn exercise_resume(workspace: std::path::PathBuf) -> serde_json::Value {
    let runtime = fixture::Runtime::start_with(&workspace, failing_then_recovering).await;
    runtime
        .command(json!({"type":"prompt","message":"Initialise the swarm","ack":"accept"}))
        .await;
    runtime.wait_report("READY").await;
    // Request 2 is the terminal failure: the coordinator suspends itself.
    let failing = runtime
        .command(json!({"type":"prompt","message":"Do the work","ack":"accept"}))
        .await;
    runtime.wait_receipt(&failing["id"], "failed").await;
    let suspended = runtime.command(json!({"type":"get_state"})).await;
    // The parent pauses (as a coordinator reflex would) and then resumes.
    let paused = runtime
        .command(json!({"type":"swarm_control","action":"pause","reason":"member failed"}))
        .await;
    let resumed = runtime
        .command(json!({"type":"swarm_control","action":"resume"}))
        .await;
    // No prompt, no steer: the resume's own wake re-arms and runs a turn.
    runtime
        .wait_state("re-armed after resume", |data| {
            data["automaticTurnsSuspended"] == false
        })
        .await;
    runtime.wait_report("RECOVERED").await;
    let restored = runtime.command(json!({"type":"get_state"})).await;
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        checkout: workspace.clone(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
    };
    context.cancel_run().unwrap();
    let evidence = json!({
        "suspended": suspended["data"]["automaticTurnsSuspended"],
        "paused": paused["data"]["status"],
        "resumed": resumed["data"]["status"],
        "resume_generation": resumed["data"]["generation"],
        "pause_generation": paused["data"]["generation"],
        "restored": restored["data"]["automaticTurnsSuspended"],
        "requests": runtime.requests.load(std::sync::atomic::Ordering::SeqCst),
    });
    runtime.finish().await;
    evidence
}

#[then("the coordinator is re-armed by the resume alone and continues its work")]
fn resumed_without_prompt(world: &mut QuectoWorld) {
    let evidence = result_json(world);
    assert_eq!(evidence["suspended"], true, "{evidence}");
    assert_eq!(evidence["paused"], "paused", "{evidence}");
    assert_eq!(evidence["resumed"], "running", "{evidence}");
    assert!(
        evidence["resume_generation"].as_u64() > evidence["pause_generation"].as_u64(),
        "{evidence}"
    );
    assert_eq!(evidence["restored"], false, "{evidence}");
    assert!(evidence["requests"].as_u64().unwrap() >= 4, "{evidence}");
}
