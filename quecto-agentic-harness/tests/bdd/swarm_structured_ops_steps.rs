//! #2279: the structured board ops as the coordinator and a worker call
//! them through their `swarm` tools.
use super::{QuectoWorld, result, result_json, run};
use cucumber::{then, when};
use serde_json::json;

/// #2279: a structured op through the coordinator's tool, which must answer.
fn op(world: &mut QuectoWorld, request: serde_json::Value) -> serde_json::Value {
    run(world, request.clone());
    assert!(
        !result(world).is_error,
        "{request}: {}",
        result(world).content
    );
    result_json(world)
}

#[when("the coordinator creates a task through a structured op")]
fn structured_task(world: &mut QuectoWorld) {
    let task = op(
        world,
        json!({"op":"task_create","request":"r1","title":"implement behavior",
            "acceptance":["tests pass"]}),
    );
    assert_eq!(task["id"], 1, "{task}");
    assert_eq!(task["status"], "ready", "{task}");
}

#[when("a worker claims, reserves and submits it through structured ops")]
fn structured_worker(world: &mut QuectoWorld) {
    use quecto::application::swarm::ports::CoordinationPort;
    use quecto::application::tools::ports::Tool;
    let workspace = world.swarm_workspace.clone().unwrap();
    let coordinator = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: workspace.clone(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    };
    coordinator.reserve_member("worker", "reservation").unwrap();
    let worker = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        member: "worker".into(),
        ..coordinator
    };
    worker
        .join(
            &quecto::domain::swarm::ProcessIdentity {
                pid: 123,
                started: "identity".into(),
            },
            None,
            Some("reservation"),
        )
        .unwrap();
    let workspace = std::sync::Arc::new(workspace);
    let tool = quecto::infrastructure::tools::swarm::SwarmTool::new(
        workspace.clone(),
        std::sync::Arc::new(quecto::infrastructure::security::sandbox::Sandbox::new(
            Some(workspace.as_ref().clone()),
        )),
        quecto::infrastructure::tools::swarm::SwarmConfig::default(),
    )
    .with_context(Some(worker));
    let call = |request: serde_json::Value| -> serde_json::Value {
        let result = super::runtime()
            .block_on(tool.execute(&request.to_string()))
            .unwrap();
        assert!(!result.is_error, "{request}: {}", result.content);
        serde_json::from_str(&result.content).unwrap()
    };
    let claim = call(json!({"op":"claim","task_id":1}));
    assert_eq!(claim["owner"], "worker", "{claim}");
    let token = claim["token"].clone();
    let reserved = call(json!({"op":"reserve","task_id":1,"token":token,"paths":["src/lib.rs"]}));
    assert_eq!(reserved["paths"], json!(["src/lib.rs"]), "{reserved}");
    let submitted = call(json!({"op":"submit","task_id":1,"token":token,
        "evidence":[{"artifact":"tests.log","revision":"R1"}]}));
    assert!(submitted.is_null(), "{submitted}");
}

#[when("the coordinator verifies the submitted task through a structured op")]
fn structured_verify(world: &mut QuectoWorld) {
    let task = op(world, json!({"op":"task","task_id":1}));
    assert_eq!(task["status"], "submitted", "{task}");
    let verified = op(
        world,
        json!({"op":"verify_task","task_id":1,"token":task["token"],"revision":"R1"}),
    );
    assert!(verified.is_null(), "{verified}");
    run(world, json!({"op":"summary"}));
}

#[then("the verified task's reservation is released")]
fn structured_reservation_released(world: &mut QuectoWorld) {
    assert_eq!(result_json(world)["files"], json!([]));
    let owners = op(world, json!({"op":"file_owners"}));
    assert_eq!(owners, json!([]));
}
