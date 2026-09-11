use super::{QuectoWorld, result, result_json, run};
use cucumber::{given, then, when};
use serde_json::json;

#[when("a swarm member creates an acceptance task")]
fn create_task(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; print(board.task_create('acceptance', 'implement behavior', ['tests pass'], []))"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[then("a later swarm execution sees the acceptance task")]
fn durable_task(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; print(board.task(1)['title'])"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    assert_eq!(
        result_json(world)["stdout"].as_str().unwrap().trim(),
        "implement behavior"
    );
}

#[when("a swarm member claims work with an unmet dependency")]
fn dependency(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; a=board.task_create('a','first',['pass'],[]); b=board.task_create('b','second',['pass'],[a['id']]); board.claim(b['id'])"}),
    );
}

#[when("a swarm member submits its work")]
fn submit(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; a=board.task_create('a','first',['pass'],[]); c=board.claim(a['id']); board.submit(a['id'],c['token'],[{'artifact':'tests.log','revision':'abc'}])"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    run(world, json!({"op":"summary"}));
}

#[then(expr = "the swarm task status is {string}")]
fn task_status(world: &mut QuectoWorld, status: String) {
    assert_eq!(result_json(world)["tasks"][0]["status"], status);
}

#[when("the swarm coordinator attempts completion without evidence")]
fn false_completion(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; board.complete('abc')"}),
    );
}

#[when("the swarm parent cancels the run")]
fn cancel(world: &mut QuectoWorld) {
    run(world, json!({"op":"cancel_run"}));
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[then(expr = "the swarm run status is {string}")]
fn run_status(world: &mut QuectoWorld, status: String) {
    assert_eq!(result_json(world)["status"], status);
}

#[then("the swarm summary retains one task")]
fn partial_summary(world: &mut QuectoWorld) {
    assert_eq!(result_json(world)["tasks"].as_array().unwrap().len(), 1);
}

#[when("swarm members contend for an overlapping file set")]
fn overlap(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board, SwarmError\na=board.task_create('a','first',['pass'],[])\nb=board.task_create('b','second',['pass'],[])\nx=board.claim(a['id']); y=board.claim(b['id'])\nboard.reserve(a['id'],x['token'],['a.rs'])\ntry:\n board.reserve(b['id'],y['token'],['free.rs','./a.rs'])\nexcept SwarmError:\n pass"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    run(world, json!({"op":"summary"}));
}

#[then("only the first swarm file set is owned")]
fn ownership(world: &mut QuectoWorld) {
    let summary = result_json(world);
    let files = summary["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "a.rs");
    assert_eq!(files[0]["task"], 1);
}

#[when("the coordinator completes dependent tasks at different revisions")]
fn dependent_revisions(world: &mut QuectoWorld) {
    run(
        world,
        json!({"code":"from swarm import board\na=board.task_create('a','first',['pass'])\nx=board.claim(a['id'])\nboard.submit(a['id'],x['token'],[{'artifact':'a.log','revision':'R1'}])\nboard.verify_task(a['id'],x['token'],'R1')\nb=board.task_create('b','second',['pass'],[a['id']])\ny=board.claim(b['id'])\nboard.submit(b['id'],y['token'],[{'artifact':'b.log','revision':'R2'}])\nboard.verify_task(b['id'],y['token'],'R2')\nboard.evidence('tests','final.log','R2','command',True)\nboard.complete('R2')"}),
    );
    assert!(result(world).is_error);
    assert!(result(world).content.contains("stale revision"));
}

#[when("revalidates earlier work with fresh final revision evidence")]
fn revalidate_revision(world: &mut QuectoWorld) {
    run(
        world,
        json!({"code":"from swarm import board\nboard.revalidate_task(1,'R2',[{'artifact':'a-rerun.log','revision':'R2'}])\nboard.complete('R2')"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    run(world, json!({"op":"summary"}));
}

#[when("a swarm member supplies acceptance as a string")]
fn acceptance_type(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; board.task_create('invalid','work','tests pass')"}),
    );
}

#[when("a swarm task awaits master approval")]
fn await_approval(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; t=board.task_create('approval','wishlist',['approved schema']); c=board.claim(t['id']); board.block(t['id'],c['token'],'awaiting master approval')"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    run(world, json!({"op":"summary"}));
}

#[when("the approved swarm task is completed")]
fn apply_approval(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; t=board.task(1); board.submit(1,t['token'],[{'artifact':'approved-tests.log','revision':'approved-revision'}])"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    run(world, json!({"op":"summary"}));
}

#[when("a swarm participant requests a workflow-enabled worker")]
async fn reject_workflow(world: &mut QuectoWorld) {
    use quecto::domain::tool::Tool;
    let tool = quecto::infrastructure::tools::spawn::SpawnTool::new(vec![])
        .with_swarm_context(Some(
            quecto::infrastructure::tools::swarm_bridge::SwarmContext {
                checkout: std::env::temp_dir(),
                member: "coordinator".into(),
                lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
            },
        ))
        .with_swarm_participation(
            quecto::infrastructure::tools::swarm_bridge::Participation::Fixed(true),
        );
    world.swarm_result = Some(
        tool.execute(r#"{"agent_id":"worker","workflow":true}"#)
            .await
            .unwrap(),
    );
}

#[when("a workflow-enabled ordinary container is requested")]
async fn ordinary_container_workflow(world: &mut QuectoWorld) {
    use quecto::domain::tool::Tool;
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
        checkout: workspace.clone(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    };
    let socket = workspace.join("worker.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    use quecto::domain::swarm::CoordinationPort;
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
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
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
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
        json!({"op":"run","code":"from swarm import board; board.send('approval','worker','Approved: schema v2')"}),
    );
    recipient.join().unwrap();
}

#[then("the rejected wake still leaves the message in the recipient inbox")]
fn durable_rejected_wake(world: &mut QuectoWorld) {
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        checkout: world.swarm_workspace.clone().unwrap(),
        member: "worker".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    };
    use quecto::domain::tool::Tool;
    let workspace = std::sync::Arc::new(context.checkout.clone());
    let tool = quecto::infrastructure::tools::swarm::SwarmTool::new(
        workspace.clone(),
        std::sync::Arc::new(quecto::infrastructure::security::sandbox::Sandbox::new(
            Some(workspace.as_ref().clone()),
        )),
        quecto::infrastructure::tools::swarm::SwarmConfig::default(),
    )
    .with_context(Some(context));
    let result = super::runtime().block_on(tool.execute(r#"{"op":"run","code":"from swarm import board; import json; print(json.dumps(board.inbox()))"}"#)).unwrap();
    assert!(!result.is_error, "{}", result.content);
    let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let messages: serde_json::Value =
        serde_json::from_str(output["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(messages[0]["body"], "Approved: schema v2");
    assert_eq!(messages[0]["status"], "accepted");
}

#[when("the member replaces submitted evidence under the same claim")]
fn replace_submission(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run", "code":"from swarm import board; t=board.task(1); board.submit(t['id'],t['token'],[{'artifact':'unreviewed.log','revision':'abc'}])"}),
    );
}

#[when("the swarm coordinator changes only the done criteria")]
fn amend_criteria(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run", "code":"from swarm import board; s=board.summary(); board.amend(s['goal'],s['constraints'],[{'id':'changed','kind':'review','description':'new requirement'}],'approved change')"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
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

#[given("swarm Python is permitted to create subprocesses")]
fn permit_subprocesses(world: &mut QuectoWorld) {
    let workspace = super::ensure_workspace(world);
    let tool = quecto::infrastructure::tools::swarm_test_support::tool(
        std::sync::Arc::new(workspace.clone()),
        std::sync::Arc::new(quecto::infrastructure::security::sandbox::Sandbox::new(
            Some(workspace),
        )),
        quecto::infrastructure::tools::swarm::SwarmConfig {
            max_processes: None,
            ..Default::default()
        },
    );
    world.swarm_tool = Some(crate::DebugSwarm(std::sync::Arc::new(tool)));
}

#[when(expr = "a {string} swarm interpreter returns before its ordinary child")]
fn child_outlives_interpreter(world: &mut QuectoWorld, mode: String) {
    assert!(matches!(mode.as_str(), "foreground" | "background"));
    let child = "import pathlib,time,os; pathlib.Path('child-ready').write_text(str(os.getpid())); time.sleep(10)";
    let code = format!(
        "import pathlib,subprocess,sys,time\nsubprocess.Popen([sys.executable,'-c',{}],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\nwhile not pathlib.Path('child-ready').exists(): time.sleep(0.01)",
        json!(child)
    );
    run(world, json!({"code":code,"background":mode=="background"}));
    assert!(!result(world).is_error, "{}", result(world).content);
    if mode == "background" {
        let id = result_json(world)["job_id"].clone();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            run(world, json!({"op":"status","job_id":id}));
            if result_json(world)["status"] == "completed" {
                break;
            }
            assert!(
                std::time::Instant::now() < until,
                "invocation did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert_eq!(result_json(world)["status"], "completed");
}

#[then("the completed swarm invocation has stopped its child")]
fn invocation_child_stopped(world: &mut QuectoWorld) {
    let workspace = super::ensure_workspace(world);
    let pid: u32 = std::fs::read_to_string(workspace.join("child-ready"))
        .unwrap()
        .parse()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let running = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(')')
                    .map(|(_, tail)| !tail.trim_start().starts_with('Z'))
            })
            .unwrap_or(false);
        if !running {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "completed interpreter left its child running"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[given("an idle swarm peer with an unavailable endpoint")]
fn idle_peer(world: &mut QuectoWorld) {
    use quecto::domain::swarm::{CoordinationPort, ProcessIdentity};
    run(world, json!({"op":"summary"}));
    let checkout = world.swarm_workspace.clone().unwrap();
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
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
    run(
        world,
        json!({"op":"run", "code":"from swarm import board; t=board.task_create('claim-now','work',['pass']); board.claim(t['id'])"}),
    );
}

#[then("no swarm wake delivery is attempted")]
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
    run(
        world,
        json!({"op":"run","code":"from swarm import board; import json; t=board.task_create('approval-resume','work',['pass']); c=board.claim(t['id']); board.reserve(t['id'],c['token'],['owned.rs']); board.block(t['id'],c['token'],'approval'); board.unblock(t['id'],c['token'],'approved'); resumed=board.task(t['id']); print(json.dumps({'before':c['token'],'after':resumed['token'],'status':resumed['status'],'files':board.summary()['files']}))"}),
    );
}

#[then("the resumed swarm task retains its claim and reserved file")]
fn unblocked_ownership(world: &mut QuectoWorld) {
    assert!(!result(world).is_error, "{}", result(world).content);
    let value: serde_json::Value =
        serde_json::from_str(result_json(world)["stdout"].as_str().unwrap()).unwrap();
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
        checkout: world.swarm_workspace.clone().unwrap(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(quecto::application::ports::SwarmTestLifecycle),
    }
}

/// Apply a supervisor control through the same port the parent's
/// `swarm_control` command uses, recording the receipt or error as the result.
fn supervise(world: &mut QuectoWorld, action: quecto::domain::swarm::RunControlAction) {
    use quecto::domain::swarm::SwarmRunControl;
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
    run(
        world,
        json!({"op":"run","code":format!("from swarm import board; a=board.task_create('a','first',['pass'],[]); board.claim(a['id']); board.stop('{status}','needs the master')")}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    run(world, json!({"op":"summary"}));
}

#[when("the coordinator completes the run with accepted evidence")]
fn coordinator_completes(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; board.evidence('tests','proof','R1','command',True); board.complete('R1')"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
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
    let status = std::process::Command::new("python3")
        .args([
            "-c",
            "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute('UPDATE run SET deadline=1'); db.commit()",
        ])
        .arg(&store)
        .status()
        .unwrap();
    assert!(status.success());
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
        json!({"op":"run","code":"from swarm import board; board.task_create('late','work',['pass'],[])"}),
    );
    assert!(result(world).is_error, "{}", result(world).content);
}

#[then("the coordinator can run Python on the board again")]
fn python_again(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; print(board.summary()['status'])"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    assert_eq!(
        result_json(world)["stdout"].as_str().unwrap().trim(),
        "running"
    );
}

// ── #1837: messages carry a revision and can be superseded ──────────────

#[when("a swarm member sends a message and then supersedes it with a newer revision")]
fn supersede_message(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; first=board.send('r1','coordinator','review head one',revision='abc1'); second=board.send('r2','coordinator','review head two',revision='abc2',supersedes=first['id']); print(board.inbox())"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[then("the recipient inbox holds only the newer message with its revision")]
fn inbox_holds_newer(world: &mut QuectoWorld) {
    let stdout = result_json(world)["stdout"].as_str().unwrap().to_string();
    assert!(stdout.contains("'revision': 'abc2'"), "{stdout}");
    assert!(!stdout.contains("'revision': 'abc1'"), "{stdout}");
    assert!(stdout.contains("'supersedes': 1,"), "{stdout}");
    assert_eq!(
        stdout.matches("'id': ").count(),
        1,
        "exactly one unread message: {stdout}"
    );
}

#[then("the superseded message remains in the audit")]
fn superseded_in_audit(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; print([(m['id'], m['status'], m['superseded_by']) for m in board.inbox(include_consumed=True)])"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    let stdout = result_json(world)["stdout"].as_str().unwrap().to_string();
    assert!(stdout.contains("(1, 'superseded', 2)"), "{stdout}");
    assert!(stdout.contains("(2, 'accepted', None)"), "{stdout}");
}
