use super::coordination_ops::{claim, create, op};
use super::{QuectoWorld, result, result_json, run};
use cucumber::{given, then, when};
use serde_json::json;

#[when("a swarm participant requests a workflow-enabled worker")]
async fn reject_workflow(world: &mut QuectoWorld) {
    use quecto::application::tools::ports::Tool;
    let tool = quecto::infrastructure::tools::spawn::SpawnTool::new(vec![])
        .with_swarm_context(Some(
            quecto::infrastructure::tools::swarm_bridge::SwarmContext {
                board: quecto::composition::swarm::swarm_board(),
                checkout: std::env::temp_dir(),
                member: "coordinator".into(),
                lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
            },
        ))
        .with_swarm_participation(
            quecto::infrastructure::tools::swarm_bridge::Participation::Fixed(true),
        );
    // A spawn refusal is a tool error (#2221), shown to the model as the
    // agent loop renders it.
    world.swarm_result = Some(
        tool.execute(r#"{"agent_id":"worker","workflow":true}"#)
            .await
            .unwrap_or_else(|error| quecto::domain::tool::ToolResult::from_error(&error)),
    );
}

#[when("a workflow-enabled ordinary container is requested")]
async fn ordinary_container_workflow(world: &mut QuectoWorld) {
    use quecto::application::tools::ports::Tool;
    // No container runtime is configured here, so the launch fails later on
    // configuration; the point is that validation no longer refuses workflow.
    let tool = quecto::infrastructure::tools::spawn::SpawnTool::new(vec![]);
    world.swarm_result = Some(
        tool.execute(r#"{"agent_id":"c2","container":true,"workflow":true}"#)
            .await
            .unwrap(),
    );
}

#[then("the swarm result should not reject workflow")]
fn not_rejected_for_workflow(world: &mut QuectoWorld) {
    let result = world.swarm_result.as_ref().unwrap();
    assert!(
        !result.content.contains("workflow is unavailable"),
        "{}",
        result.content
    );
}

fn engaged_workflow() -> quecto::infrastructure::tools::swarm_bridge::WorkflowEngineSlot {
    let slot: quecto::infrastructure::tools::swarm_bridge::WorkflowEngineSlot = Default::default();
    let _ = slot.set(std::sync::Arc::new(std::sync::Mutex::new(
        quecto::domain::workflow::WorkflowEngine::new(
            quecto::domain::workflow::WorkflowConfig::default(),
            true,
        )
        .unwrap(),
    )));
    slot
}

#[when("a workflow-enabled agent tries to create a swarm run")]
fn workflow_agent_creates(world: &mut QuectoWorld) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    // The create step runs on a blocking pool, so it needs a Tokio runtime.
    let outcome = tokio::runtime::Runtime::new().unwrap().block_on(
        quecto::infrastructure::tools::swarm_control::control_with_workflow(
            context,
            "create",
            json!({"goal":"g","constraints":[],
                "criteria":[{"id":"t","kind":"command","description":"pass"}],
                "member_limit":2,"deadline":deadline}),
            quecto::infrastructure::tools::swarm_bridge::Participation::shared(),
            engaged_workflow(),
        ),
    );
    let is_error = outcome.is_err();
    let content = match outcome {
        Ok(value) => value.to_string(),
        Err(error) => error.to_string(),
    };
    world.swarm_result = Some(quecto::domain::tool::ToolResult {
        content,
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    });
}

#[when("a swarm message recipient rejects its wake hint")]
fn rejected_wake(world: &mut QuectoWorld) {
    use std::io::Write;
    run(world, json!({"op":"summary"}));
    let workspace = world.swarm_workspace.clone().unwrap();
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: workspace.clone(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    };
    let socket = workspace.join("worker.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    use quecto::application::swarm::ports::CoordinationPort;
    context.reserve_member("worker", "reservation").unwrap();
    let worker = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        member: "worker".into(),
        ..context
    };
    worker
        .join(
            &quecto::domain::swarm::ProcessIdentity {
                pid: 123,
                started: "identity".into(),
            },
            socket.to_str(),
            Some("reservation"),
        )
        .unwrap();
    listener.set_nonblocking(true).unwrap();
    let recipient = std::thread::spawn(move || {
        // The hint is sent after the `send` op below; under a fully loaded
        // coverage run (24 shards) that takes seconds, so the bound is
        // generous while a missing hint still fails.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                Err(e) => panic!("wake hint not delivered: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .unwrap();
        let command = quecto::infrastructure::test_support::read_framed_command(&stream).unwrap();
        let command: serde_json::Value = serde_json::from_str(&command).unwrap();
        assert_eq!(command["type"], "swarm_control");
        assert_eq!(command["action"], "wake");
        assert!(
            command["generation"]
                .as_u64()
                .is_some_and(|value| value > 0)
        );
        writeln!(stream,"{}",json!({"type":"response","id":command["id"],"success":false,"error":"control queue is full"})).unwrap();
    });
    run(
        world,
        json!({"op":"send","request":"approval","recipient":"worker","body":"Approved: schema v2"}),
    );
    recipient.join().unwrap();
}

#[then("the rejected wake still leaves the message in the recipient inbox")]
fn durable_rejected_wake(world: &mut QuectoWorld) {
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: world.swarm_workspace.clone().unwrap(),
        member: "worker".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    };
    use quecto::application::tools::ports::Tool;
    let tool = quecto::infrastructure::tools::swarm::SwarmTool::new().with_context(Some(context));
    let result = super::runtime()
        .block_on(tool.execute(r#"{"op":"inbox"}"#))
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let messages: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(messages[0]["body"], "Approved: schema v2");
    assert_eq!(messages[0]["status"], "accepted");
}

#[when("the member replaces submitted evidence under the same claim")]
fn replace_submission(world: &mut QuectoWorld) {
    let task = op(world, json!({"op":"task","task_id":1}));
    run(
        world,
        json!({"op":"submit","task_id":task["id"],"token":task["token"],
            "evidence":[{"artifact":"unreviewed.log","revision":"abc"}]}),
    );
}

#[when("the swarm coordinator changes only the done criteria")]
fn amend_criteria(world: &mut QuectoWorld) {
    let summary = op(world, json!({"op":"summary"}));
    op(
        world,
        json!({"op":"amend","goal":summary["goal"],"constraints":summary["constraints"],
            "criteria":[{"id":"changed","kind":"review","description":"new requirement"}],
            "reason":"approved change"}),
    );
    run(world, json!({"op":"summary"}));
}

#[then("the swarm audit retains both complete contracts")]
fn contract_history(world: &mut QuectoWorld) {
    let criteria = result_json(world)["criteria"].clone();
    run(world, json!({"op":"events","limit":100}));
    let summary = result_json(world);
    let events = summary["events"].as_array().unwrap();
    let detail = |action: &str| -> serde_json::Value {
        serde_json::from_str(
            events.iter().find(|e| e["action"] == action).unwrap()["detail"]
                .as_str()
                .unwrap(),
        )
        .unwrap()
    };
    let created = detail("created");
    let amended = detail("amended");
    assert_eq!(amended["before"], created["contract"]);
    assert_eq!(amended["after"]["criteria"], criteria);
    assert_ne!(amended["before"]["criteria"], amended["after"]["criteria"]);
    assert_eq!(amended["reason"], "approved change");
}

#[given("an idle swarm peer with an unavailable endpoint")]
fn idle_peer(world: &mut QuectoWorld) {
    use quecto::application::swarm::ports::CoordinationPort;
    use quecto::domain::swarm::ProcessIdentity;
    run(world, json!({"op":"summary"}));
    let checkout = world.swarm_workspace.clone().unwrap();
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: checkout.clone(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    };
    context
        .reserve_member("idle-peer", "idle-reservation")
        .unwrap();
    let worker = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        member: "idle-peer".into(),
        ..context
    };
    worker
        .join(
            &ProcessIdentity {
                pid: 123,
                started: "fixture".into(),
            },
            checkout.join("unavailable.sock").to_str(),
            Some("idle-reservation"),
        )
        .unwrap();
}

#[when("the coordinator creates and immediately claims a task")]
fn immediately_claim(world: &mut QuectoWorld) {
    let task = op(
        world,
        json!({"op":"task_create","request":"claim-now","title":"work","acceptance":["pass"]}),
    );
    // #2281: a ready task wakes the idle peer after its own op (S14 wakes
    // after each mutating op; one `op=run` program created and claimed
    // before any wake); only the claim is the subject below.
    assert_eq!(
        task["notification_warnings"],
        json!(["wake hint failed for idle-peer; durable board is authoritative"]),
        "{task}"
    );
    run(world, json!({"op":"claim","task_id":task["id"]}));
}

#[then("the claim attempts no swarm wake delivery")]
fn no_idle_wake(world: &mut QuectoWorld) {
    assert!(!result(world).is_error, "{}", result(world).content);
    assert!(
        result_json(world).get("notification_warnings").is_none(),
        "a stale hint attempted delivery to the unavailable peer: {}",
        result(world).content
    );
}

#[when("an owned blocked swarm task is unblocked")]
fn unblock_owned_task(world: &mut QuectoWorld) {
    let task = op(
        world,
        json!({"op":"task_create","request":"approval-resume","title":"work","acceptance":["pass"]}),
    );
    let id = task["id"].clone();
    let token = claim(world, &id);
    op(
        world,
        json!({"op":"reserve","task_id":id,"token":token,"paths":["owned.rs"]}),
    );
    op(
        world,
        json!({"op":"block","task_id":id,"token":token,"reason":"approval"}),
    );
    op(
        world,
        json!({"op":"unblock","task_id":id,"token":token,"reason":"approved"}),
    );
    let resumed = op(world, json!({"op":"task","task_id":id}));
    let files = op(world, json!({"op":"summary"}))["files"].clone();
    let view =
        json!({"before":token,"after":resumed["token"],"status":resumed["status"],"files":files});
    world.swarm_result = Some(quecto::domain::tool::ToolResult {
        content: view.to_string(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    });
}

#[then("the resumed swarm task retains its claim and reserved file")]
fn unblocked_ownership(world: &mut QuectoWorld) {
    assert!(!result(world).is_error, "{}", result(world).content);
    let value = result_json(world);
    assert_eq!(value["before"], value["after"]);
    assert_eq!(value["status"], "claimed");
    assert_eq!(value["files"][0]["path"], "owned.rs");
}

#[when("the supervisor durably pauses the swarm")]
fn durable_pause(world: &mut QuectoWorld) {
    run(world, json!({"op":"pause", "reason":"master approval"}));
    run(world, json!({"op":"summary"}));
}

#[when("the supervisor resumes the swarm")]
fn durable_resume(world: &mut QuectoWorld) {
    // Only the supervisor outside the swarm resumes (#1729); the member-side
    // `swarm resume` op is refused.
    supervise(world, quecto::domain::swarm::RunControlAction::Resume);
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[when("the supervisor inspects the unchanged swarm cursor")]
fn unchanged_inspection(world: &mut QuectoWorld) {
    let cursor = result_json(world)["event_cursor"].clone();
    run(world, json!({"op":"summary", "since":cursor}));
}

#[then("the swarm inspection is unchanged without task history")]
fn unchanged_board(world: &mut QuectoWorld) {
    let value = result_json(world);
    assert_eq!(value["unchanged"], true);
    assert!(value.get("tasks").is_none());
    assert!(value.get("events").is_none());
    assert!(result(world).content.len() < 150);
}

// ── #1729: every end is a pause only the supervisor lifts ────────────────

fn supervisor_context(
    world: &QuectoWorld,
) -> quecto::infrastructure::tools::swarm_bridge::SwarmContext {
    quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        checkout: world.swarm_workspace.clone().unwrap(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    }
}

/// Apply a supervisor control through the same port the parent's
/// `swarm_control` command uses, recording the receipt or error as the result.
fn supervise(world: &mut QuectoWorld, action: quecto::domain::swarm::RunControlAction) {
    use quecto::application::swarm::ports::SwarmRunControl;
    let context = supervisor_context(world);
    let outcome = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(context.apply(action))
    })
    .join()
    .unwrap();
    let (content, is_error) = match outcome {
        Ok(receipt) => (
            json!({"status": format!("{:?}", receipt.status).to_lowercase(), "generation": receipt.generation})
                .to_string(),
            false,
        ),
        Err(error) => (error.to_string(), true),
    };
    world.swarm_result = Some(quecto::domain::tool::ToolResult {
        content,
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    });
    if !is_error {
        run(world, json!({"op":"summary"}));
    }
}

#[when(expr = "the coordinator stops the run as {string}")]
fn coordinator_stops(world: &mut QuectoWorld, status: String) {
    let id = create(world, "first", json!([]));
    claim(world, &id);
    op(
        world,
        json!({"op":"stop","status":status,"reason":"needs the master"}),
    );
    run(world, json!({"op":"summary"}));
}

#[when("the coordinator completes the run with accepted evidence")]
fn coordinator_completes(world: &mut QuectoWorld) {
    op(
        world,
        json!({"op":"evidence","criterion":"tests","artifact":"proof","revision":"R1",
            "kind":"command","passed":true}),
    );
    op(world, json!({"op":"complete","revision":"R1"}));
    run(world, json!({"op":"summary"}));
}

#[when("the swarm deadline has passed")]
fn deadline_passed(world: &mut QuectoWorld) {
    // The first tool use creates the run; then age its deadline in place.
    run(world, json!({"op":"summary"}));
    assert!(!result(world).is_error, "{}", result(world).content);
    let store = world
        .swarm_workspace
        .clone()
        .unwrap()
        .join(".quecto")
        .join("swarm.sqlite");
    let board = rusqlite::Connection::open(&store).unwrap();
    assert_eq!(
        board.execute("UPDATE run SET deadline=1", []).unwrap(),
        1,
        "the board holds one run"
    );
    drop(board);
    run(world, json!({"op":"summary"}));
}

#[then(expr = "the swarm run is paused holding {string}")]
fn paused_holding(world: &mut QuectoWorld, outcome: String) {
    let value = result_json(world);
    assert_eq!(value["status"], "paused", "{value}");
    assert_eq!(value["outcome"], outcome, "{value}");
}

#[then("every swarm member is still live")]
fn members_live(world: &mut QuectoWorld) {
    let value = result_json(world);
    let members = value["members"].as_array().unwrap();
    assert!(!members.is_empty());
    assert!(members.iter().all(|m| m["status"] == "live"), "{value}");
}

#[when("a swarm member tries to resume the run")]
fn member_resumes(world: &mut QuectoWorld) {
    run(world, json!({"op":"resume"}));
    assert!(result(world).is_error, "{}", result(world).content);
}

#[when("the supervisor outside the swarm resumes the run")]
fn supervisor_resumes(world: &mut QuectoWorld) {
    supervise(world, quecto::domain::swarm::RunControlAction::Resume);
}

#[when("the supervisor outside the swarm closes the run")]
fn supervisor_closes(world: &mut QuectoWorld) {
    supervise(world, quecto::domain::swarm::RunControlAction::Close);
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[when(expr = "the supervisor outside the swarm extends the deadline by {int} seconds")]
fn supervisor_extends(world: &mut QuectoWorld, seconds: u64) {
    supervise(
        world,
        quecto::domain::swarm::RunControlAction::ExtendDeadline { seconds },
    );
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[then("a swarm member can no longer create work")]
fn no_new_work(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"task_create","request":"late","title":"work","acceptance":["pass"],
            "dependencies":[]}),
    );
    assert!(result(world).is_error, "{}", result(world).content);
}

#[then("the coordinator can act on the board again")]
fn acts_again(world: &mut QuectoWorld) {
    let task = op(
        world,
        json!({"op":"task_create","request":"again","title":"work","acceptance":["pass"]}),
    );
    assert_eq!(task["status"], "ready", "{task}");
    assert_eq!(op(world, json!({"op":"summary"}))["status"], "running");
}

// ── #1837: messages carry a revision and can be superseded ──────────────

#[when("a swarm member sends a message and then supersedes it with a newer revision")]
fn supersede_message(world: &mut QuectoWorld) {
    let first = op(
        world,
        json!({"op":"send","request":"r1","recipient":"coordinator","body":"review head one",
            "revision":"abc1"}),
    );
    op(
        world,
        json!({"op":"send","request":"r2","recipient":"coordinator","body":"review head two",
            "revision":"abc2","supersedes":first["id"]}),
    );
    run(world, json!({"op":"inbox"}));
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[then("the recipient inbox holds only the newer message with its revision")]
fn inbox_holds_newer(world: &mut QuectoWorld) {
    let inbox = result_json(world);
    let messages = inbox.as_array().unwrap();
    assert_eq!(messages.len(), 1, "exactly one unread message: {inbox}");
    assert_eq!(messages[0]["revision"], "abc2", "{inbox}");
    assert_eq!(messages[0]["supersedes"], 1, "{inbox}");
    assert!(
        messages.iter().all(|message| message["revision"] != "abc1"),
        "{inbox}"
    );
}

#[then("the superseded message remains in the audit")]
fn superseded_in_audit(world: &mut QuectoWorld) {
    let inbox = op(world, json!({"op":"inbox","include_consumed":true}));
    let rows: Vec<serde_json::Value> = inbox
        .as_array()
        .unwrap()
        .iter()
        .map(|m| json!([m["id"], m["status"], m["superseded_by"]]))
        .collect();
    assert!(rows.contains(&json!([1, "superseded", 2])), "{inbox}");
    assert!(rows.contains(&json!([2, "accepted", null])), "{inbox}");
}
