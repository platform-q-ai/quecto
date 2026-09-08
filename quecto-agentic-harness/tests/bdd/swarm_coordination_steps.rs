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

#[when("a workflow-enabled swarm container is requested")]
async fn reject_workflow(world: &mut QuectoWorld) {
    use quecto::domain::tool::Tool;
    let tool = quecto::infrastructure::tools::spawn::SpawnTool::new(vec![]);
    world.swarm_result = Some(
        tool.execute(r#"{"container":true,"workflow":true}"#)
            .await
            .unwrap(),
    );
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
        assert_eq!(command["type"], "prompt");
        assert!(command["message"].as_str().unwrap().contains("op=summary"));
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
    assert_eq!(amended["after"]["criteria"], summary["criteria"]);
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
