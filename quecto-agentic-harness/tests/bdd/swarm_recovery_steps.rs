//! #1961: a member's authoritative exit is a confirmed death (the run keeps
//! running and its work is recoverable), an unobserved loss is recorded once
//! and never re-pauses a resumed run, and `revoke` reassigns work a live
//! member will not finish. Real coordinator and member harness processes.
use super::supervision_steps::fixture::{self, Reply, Turn};
use super::{QuectoWorld, result_json};
use cucumber::{then, when};
use quecto::infrastructure::tools::swarm_bridge::SwarmContext;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const WORKER_TASK: &str = "WORKER: claim the work";
const WORKER_CLAIM: &str = "from swarm import board\nt=board.task_create('w','work',['pass'],[])\nc=board.claim(t['id'])\nboard.reserve(t['id'],c['token'],['src/a.rs'])\nprint('CLAIMED')";

/// One fake provider for the coordinator and the member it launches: the
/// coordinator spawns, kills or reconciles on request; the member claims
/// and reserves work, then idles.
fn recovery_script(turn: &Turn) -> Reply {
    let user = turn.user.as_str();
    let call = |name, arguments| Reply::ToolCall { name, arguments };
    match (user, turn.after_tool) {
        (u, _) if u.contains("Initialise") => Reply::Text("READY"),
        (u, false) if u.contains("Launch the worker") => {
            call("spawn", json!({"agent_id":"worker","task":WORKER_TASK}))
        }
        (u, true) if u.contains("Launch the worker") => Reply::Text("LAUNCHED"),
        (u, false) if u.contains(WORKER_TASK) => {
            call("swarm", json!({"op":"run","code":WORKER_CLAIM}))
        }
        (u, true) if u.contains(WORKER_TASK) => Reply::Text("CLAIMED"),
        (u, false) if u.contains("Kill the worker") => {
            call("agent_cmd", json!({"agent_id":"worker","command":"kill"}))
        }
        (u, true) if u.contains("Kill the worker") => Reply::Text("KILLED"),
        (u, false) if u.contains("Reconcile") => call("swarm", json!({"op":"reconcile"})),
        (u, true) if u.contains("Reconcile") => Reply::Text("RECONCILED"),
        // Wake nudges, exit notes and anything else: nothing to do.
        _ => Reply::Text("IDLE"),
    }
}

fn context(workspace: &Path, member: &str) -> SwarmContext {
    SwarmContext {
        checkout: workspace.to_path_buf(),
        member: member.into(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
    }
}

/// Run board Python as `member` through the real swarm tool; the stdout.
async fn python(workspace: &Path, member: &str, code: &str) -> Result<String, String> {
    use quecto::domain::tool::Tool;
    let root = std::sync::Arc::new(workspace.to_path_buf());
    let tool = quecto::infrastructure::tools::swarm::SwarmTool::new(
        root.clone(),
        std::sync::Arc::new(quecto::infrastructure::security::sandbox::Sandbox::new(
            Some(root.as_ref().clone()),
        )),
        quecto::infrastructure::tools::swarm::SwarmConfig::default(),
    )
    .with_context(Some(context(workspace, member)));
    let result = tool
        .execute(&json!({"op":"run","code":code}).to_string())
        .await
        .map_err(|e| e.to_string())?;
    if result.is_error {
        return Err(result.content);
    }
    let output: Value = serde_json::from_str(&result.content).map_err(|e| e.to_string())?;
    if output["exit_code"] != 0 {
        return Err(output.to_string());
    }
    Ok(output["stdout"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned())
}

async fn wait_summary(
    runtime: &fixture::Runtime,
    workspace: &Path,
    what: &str,
    accept: impl Fn(&Value) -> bool,
) -> Value {
    let context = context(workspace, "coordinator");
    let until = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let summary = context.summary().unwrap();
        if accept(&summary) {
            return summary;
        }
        if tokio::time::Instant::now() >= until {
            let report = runtime
                .command(json!({"type":"get_messages","count":6}))
                .await;
            panic!(
                "board never became {what}: {summary}\nevents: {:?}\nreport: {report}\n--- coordinator stderr ---\n{}",
                events(workspace),
                runtime.stderr_tail()
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn member(summary: &Value, id: &str) -> Value {
    summary["members"]
        .as_array()
        .and_then(|members| members.iter().find(|m| m["id"] == id))
        .cloned()
        .unwrap_or(Value::Null)
}

/// The member the coordinator launched: every member but the coordinator
/// and the test's own `other` claimant.
fn launched_member(summary: &Value) -> Option<Value> {
    summary["members"].as_array().and_then(|members| {
        members
            .iter()
            .find(|m| m["id"] != "coordinator" && m["id"] != "other")
            .cloned()
    })
}

/// A second live member that can claim the reopened work.
async fn admit_other(workspace: &Path) {
    let started =
        quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap();
    let code = format!(
        "from swarm import board\nboard._admit('other','reservation-other')\nboard._activate('other','reservation-other',{},{started:?},'/tmp/other.sock')",
        std::process::id()
    );
    python(workspace, "coordinator", &code).await.unwrap();
}

async fn control_status(runtime: &fixture::Runtime) -> Value {
    runtime
        .command(json!({"type":"swarm_control","action":"status"}))
        .await["data"]
        .clone()
}

fn events(workspace: &Path) -> Vec<Value> {
    context(workspace, "coordinator").events(0, 100).unwrap()["events"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn count(events: &[Value], action: &str) -> usize {
    events.iter().filter(|e| e["action"] == action).count()
}

fn spawn_exercise(
    world: &mut QuectoWorld,
    exercise: fn(PathBuf) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send>>,
) {
    let workspace = world.swarm_workspace.clone().unwrap();
    let evidence = std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(exercise(workspace))
    })
    .join()
    .unwrap_or_else(|payload| {
        let text = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "opaque panic".into());
        panic!("recovery exercise failed: {text}")
    });
    world.swarm_result = Some(quecto::domain::tool::ToolResult {
        content: evidence.to_string(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    });
}

// ── Observed exit: confirmed death, run keeps running, recover ─────────────

#[when("a member the coordinator launched is killed while holding a claim")]
fn kill_claiming_member(world: &mut QuectoWorld) {
    spawn_exercise(world, |workspace| Box::pin(exercise_kill(workspace)));
}

async fn exercise_kill(workspace: PathBuf) -> Value {
    let runtime = fixture::Runtime::start_scripted(&workspace, recovery_script).await;
    runtime
        .command(json!({"type":"prompt","message":"Initialise the swarm","ack":"accept"}))
        .await;
    runtime.wait_report("READY").await;
    runtime
        .command(json!({"type":"prompt","message":"Launch the worker","ack":"accept"}))
        .await;
    // The launched member (a real harness) claims and reserves on its own.
    let claimed = wait_summary(
        &runtime,
        &workspace,
        "claimed by the launched member",
        |s| {
            s["tasks"][0]["status"] == "claimed"
                && s["tasks"][0]["owner"] != "coordinator"
                && s["files"].as_array().is_some_and(|f| f.len() == 1)
        },
    )
    .await;
    let worker = launched_member(&claimed).unwrap();
    assert_eq!(worker["status"], "live", "{claimed}");
    assert_eq!(claimed["tasks"][0]["owner"], worker["id"], "{claimed}");
    let events_before = events(&workspace);
    runtime
        .command(json!({"type":"prompt","message":"Kill the worker","ack":"accept"}))
        .await;
    // The coordinator's reaper observed the exit: death confirmed, work
    // blocked for recovery, reservations gone, run still running.
    let dead = wait_summary(
        &runtime,
        &workspace,
        "member dead and its work blocked",
        |s| {
            launched_member(s).is_some_and(|m| m["status"] == "dead")
                && s["tasks"][0]["status"] == "blocked"
        },
    )
    .await;
    let status = control_status(&runtime).await;
    let after = events(&workspace);
    python(
        &workspace,
        "coordinator",
        "from swarm import board\nboard.recover(1)",
    )
    .await
    .unwrap();
    admit_other(&workspace).await;
    let reclaimed = python(
        &workspace,
        "other",
        "from swarm import board\nprint(board.claim(1)['owner'])",
    )
    .await
    .unwrap();
    let evidence = json!({
        "worker": worker["id"],
        "run_status": dead["status"],
        "outcome": dead["outcome"],
        "task": dead["tasks"][0],
        "files": dead["files"],
        "control_status": status["status"],
        "resume_blockers": status["resume_blockers"],
        "death_confirmed": count(&after, "death_confirmed") - count(&events_before, "death_confirmed"),
        "scope_unknown": count(&after, "scope_unknown"),
        "paused": count(&after, "paused"),
        "reclaimed_by": reclaimed,
    });
    context(&workspace, "coordinator").cancel_run().unwrap();
    runtime.finish().await;
    evidence
}

#[then("the run kept running, the member is dead and another member takes over its recovered work")]
fn recovered_after_kill(world: &mut QuectoWorld) {
    let evidence = result_json(world);
    assert_eq!(evidence["run_status"], "running", "{evidence}");
    assert_eq!(evidence["outcome"], Value::Null, "{evidence}");
    assert_eq!(evidence["control_status"], "running", "{evidence}");
    assert!(
        evidence["resume_blockers"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{evidence}"
    );
    assert_eq!(evidence["task"]["status"], "blocked", "{evidence}");
    assert_eq!(evidence["task"]["owner"], evidence["worker"], "{evidence}");
    assert_eq!(
        evidence["task"]["blocker"], "worker death confirmed; coordinator recovery required",
        "{evidence}"
    );
    assert_eq!(evidence["files"], json!([]), "{evidence}");
    assert_eq!(evidence["death_confirmed"], 1, "{evidence}");
    assert_eq!(evidence["scope_unknown"], 0, "{evidence}");
    assert_eq!(evidence["paused"], 0, "{evidence}");
    assert_eq!(evidence["reclaimed_by"], "other", "{evidence}");
}

// ── Unobserved loss: recorded once, never re-pauses a resumed run ──────────

#[when("an unobserved member loss pauses the run and the supervisor resumes it")]
fn unobserved_loss(world: &mut QuectoWorld) {
    spawn_exercise(world, |workspace| Box::pin(exercise_loss(workspace)));
}

async fn exercise_loss(workspace: PathBuf) -> Value {
    let runtime = fixture::Runtime::start_scripted(&workspace, recovery_script).await;
    runtime
        .command(json!({"type":"prompt","message":"Initialise the swarm","ack":"accept"}))
        .await;
    runtime.wait_report("READY").await;
    // A member nobody in the coordinator's harness launched: its harness is
    // a real process the store knows by pid and start time, and its claim
    // is a real one.
    let mut ghost = std::process::Command::new("sleep")
        .arg("300")
        .spawn()
        .unwrap();
    let pid = ghost.id();
    let started = quecto::infrastructure::tools::swarm_bridge::process_start(pid).unwrap();
    python(
        &workspace,
        "coordinator",
        &format!("from swarm import board\nboard._admit('ghost','reservation-ghost')\nboard._activate('ghost','reservation-ghost',{pid},{started:?},'/tmp/ghost.sock')"),
    )
    .await
    .unwrap();
    python(&workspace, "ghost", WORKER_CLAIM).await.unwrap();
    // Its harness vanishes without anyone observing the exit.
    ghost.kill().unwrap();
    ghost.wait().unwrap();
    runtime
        .command(json!({"type":"prompt","message":"Reconcile the board","ack":"accept"}))
        .await;
    let paused = wait_summary(&runtime, &workspace, "paused by the loss", |s| {
        s["status"] == "paused"
    })
    .await;
    let resumed = runtime
        .command(json!({"type":"swarm_control","action":"resume"}))
        .await["data"]
        .clone();
    runtime.wait_idle().await;
    // The same vanished pid is observed again after the resume: the
    // coordinator's reconcile (a tool call and its answer) runs to the end.
    let before = runtime.requests.load(std::sync::atomic::Ordering::SeqCst);
    runtime
        .command(json!({"type":"prompt","message":"Reconcile the board again","ack":"accept"}))
        .await;
    let until = tokio::time::Instant::now() + Duration::from_secs(15);
    while runtime.requests.load(std::sync::atomic::Ordering::SeqCst) < before + 2 {
        assert!(
            tokio::time::Instant::now() < until,
            "second reconcile never ran"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    runtime.wait_idle().await;
    let after = context(&workspace, "coordinator").summary().unwrap();
    let status = control_status(&runtime).await;
    let recover = python(
        &workspace,
        "coordinator",
        "from swarm import board\nboard.recover(1)",
    )
    .await
    .err()
    .unwrap_or_default();
    python(
        &workspace,
        "coordinator",
        "from swarm import board\nboard.revoke(1, 'ghost harness lost; reassigning')",
    )
    .await
    .unwrap();
    admit_other(&workspace).await;
    let reclaimed = python(
        &workspace,
        "other",
        "from swarm import board\nprint(board.claim(1)['owner'])",
    )
    .await
    .unwrap();
    let events = events(&workspace);
    let evidence = json!({
        "paused": paused["status"],
        "paused_outcome": paused["outcome"],
        "ghost_after_pause": member(&paused, "ghost")["status"],
        "resumed": resumed["status"],
        "after": after["status"],
        "after_outcome": after["outcome"],
        "control_status": status["status"],
        "resume_blockers": status["resume_blockers"],
        "scope_unknown": count(&events, "scope_unknown"),
        "pauses": count(&events, "paused"),
        "recover_refusal": recover,
        "revoked": count(&events, "revoked"),
        "reclaimed_by": reclaimed,
    });
    context(&workspace, "coordinator").cancel_run().unwrap();
    runtime.finish().await;
    evidence
}

#[then("the stale loss never pauses the run again and the work is revoked and reclaimed")]
fn no_repause(world: &mut QuectoWorld) {
    let evidence = result_json(world);
    assert_eq!(evidence["paused"], "paused", "{evidence}");
    assert_eq!(evidence["paused_outcome"], "failed", "{evidence}");
    // An unobserved loss retains the member and its ownership (#1924 rule).
    assert_eq!(evidence["ghost_after_pause"], "live", "{evidence}");
    assert_eq!(evidence["resumed"], "running", "{evidence}");
    assert_eq!(evidence["after"], "running", "{evidence}");
    assert_eq!(evidence["after_outcome"], Value::Null, "{evidence}");
    assert_eq!(evidence["control_status"], "running", "{evidence}");
    assert!(
        evidence["resume_blockers"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{evidence}"
    );
    assert_eq!(evidence["scope_unknown"], 1, "{evidence}");
    assert_eq!(evidence["pauses"], 1, "{evidence}");
    assert!(
        evidence["recover_refusal"]
            .as_str()
            .is_some_and(|text| text.contains("revoke")),
        "{evidence}"
    );
    assert_eq!(evidence["revoked"], 1, "{evidence}");
    assert_eq!(evidence["reclaimed_by"], "other", "{evidence}");
}

// ── Revoke: a live owner's claim reassigned by the coordinator ─────────────

/// The world's own swarm tool acts as the coordinator; other members act
/// through their own tool instance on the same store.
fn python_as(world: &mut QuectoWorld, member: &str, code: &str) -> Result<String, String> {
    let workspace = world.swarm_workspace.clone().unwrap();
    super::runtime().block_on(python(&workspace, member, code))
}

#[when("a suspended member holds a claimed task with a reserved file")]
fn suspended_member_claims(world: &mut QuectoWorld) {
    // The world's tool creates the store as the coordinator on first use.
    super::run(world, json!({"op":"summary"}));
    let started =
        quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap();
    python_as(
        world,
        "coordinator",
        &format!(
            "from swarm import board\nboard._admit('worker','reservation-worker')\nboard._activate('worker','reservation-worker',{},{started:?},'/tmp/worker.sock')",
            std::process::id()
        ),
    )
    .unwrap();
    let token = python_as(
        world,
        "worker",
        "from swarm import board\nt=board.task_create('w','work',['pass'],[])\nc=board.claim(t['id'])\nboard.reserve(t['id'],c['token'],['src/a.rs'])\nprint(c['token'])",
    )
    .unwrap();
    world.swarm_claim_token = Some(token);
}

#[when(expr = "the coordinator revokes that claim as {string}")]
fn coordinator_revokes(world: &mut QuectoWorld, reason: String) {
    super::run(
        world,
        json!({"op":"run","code":format!("from swarm import board\nimport json\nprint(json.dumps(board.revoke(1, {reason:?})))")}),
    );
    assert!(
        !super::result(world).is_error,
        "{}",
        super::result(world).content
    );
}

#[when("a member other than the coordinator tries to revoke that claim")]
fn member_revokes(world: &mut QuectoWorld) {
    let outcome = python_as(
        world,
        "worker",
        "from swarm import board\nboard.revoke(1, 'not mine to take')",
    );
    world.swarm_result = Some(quecto::domain::tool::ToolResult {
        content: outcome.clone().unwrap_or_else(|error| error),
        is_error: outcome.is_err(),
        image_blocks: vec![],
        delivery_metadata: None,
    });
}

#[then("the revoked task is ready without owner, reservation or evidence")]
fn revoked_task_ready(world: &mut QuectoWorld) {
    let task: Value = serde_json::from_str(result_json(world)["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(task["status"], "ready", "{task}");
    assert_eq!(task["owner"], Value::Null, "{task}");
    assert_eq!(task["token"], Value::Null, "{task}");
    assert_eq!(task["evidence"], json!([]), "{task}");
    super::run(world, json!({"op":"summary"}));
    let summary = result_json(world);
    assert_eq!(summary["files"], json!([]), "{summary}");
    assert_eq!(summary["status"], "running", "{summary}");
}

#[then("the revocation is audited with its reason and previous owner")]
fn revocation_audited(world: &mut QuectoWorld) {
    let workspace = world.swarm_workspace.clone().unwrap();
    let revoked: Vec<Value> = events(&workspace)
        .into_iter()
        .filter(|e| e["action"] == "revoked")
        .collect();
    assert_eq!(revoked.len(), 1, "{revoked:?}");
    let detail: Value = serde_json::from_str(revoked[0]["detail"].as_str().unwrap()).unwrap();
    assert_eq!(
        detail,
        json!({"task":1,"reason":"member suspended by provider","previous_owner":"worker"})
    );
}

#[then("the previous owner is told its claim was revoked")]
fn previous_owner_told(world: &mut QuectoWorld) {
    let inbox = python_as(
        world,
        "worker",
        "from swarm import board\nimport json\nprint(json.dumps(board.inbox()))",
    )
    .unwrap();
    let inbox: Vec<Value> = serde_json::from_str(&inbox).unwrap();
    assert_eq!(inbox.len(), 1, "{inbox:?}");
    assert_eq!(inbox[0]["sender"], "coordinator");
    let body = inbox[0]["body"].as_str().unwrap();
    assert!(body.contains("task 1 revoked"), "{body}");
    assert!(body.contains("member suspended by provider"), "{body}");
}

#[when("another member claims and completes the revoked task")]
fn other_member_completes(world: &mut QuectoWorld) {
    let workspace = world.swarm_workspace.clone().unwrap();
    super::runtime().block_on(admit_other(&workspace));
    python_as(
        world,
        "other",
        "from swarm import board\nc=board.claim(1)\nboard.submit(1,c['token'],[{'artifact':'tests.log','revision':'r2'}])",
    )
    .unwrap();
    super::run(
        world,
        json!({"op":"run","code":"from swarm import board\nboard.verify_task(1, board.task(1)['token'], 'r2')"}),
    );
    assert!(
        !super::result(world).is_error,
        "{}",
        super::result(world).content
    );
    super::run(world, json!({"op":"summary"}));
}

#[then("the revoked owner's stale token can no longer act on the task")]
fn stale_token_refused(world: &mut QuectoWorld) {
    let token = world.swarm_claim_token.clone().unwrap();
    let outcome = python_as(
        world,
        "worker",
        &format!(
            "from swarm import board\nboard.submit(1, {token:?}, [{{'artifact':'late.log','revision':'r1'}}])"
        ),
    );
    let error = outcome.expect_err("a revoked token must not submit");
    assert!(error.contains("stale or unowned claim"), "{error}");
}
