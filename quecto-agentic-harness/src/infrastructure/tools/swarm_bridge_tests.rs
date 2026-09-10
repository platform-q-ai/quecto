use super::swarm::{SwarmConfig, SwarmTool};
use super::swarm_bridge::{SwarmContext, process_confirmed_dead, process_start};
use crate::domain::tool::Tool;
use crate::infrastructure::security::sandbox::Sandbox;
use serde_json::json;
use std::sync::Arc;

fn context(directory: &tempfile::TempDir) -> SwarmContext {
    SwarmContext {
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
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
fn harness_death_alone_retains_a_recorded_launch() {
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
        2
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

#[tokio::test]
async fn ready_failure_retains_launched_scope_after_child_rollback() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 2);
    let mut reservation =
        super::swarm_admission::LaunchReservation::reserve(context.clone()).unwrap();
    let mut command = tokio::process::Command::new("sleep");
    command.arg("30").kill_on_drop(true);
    let child = command.spawn().unwrap();
    reservation.launched(child.id().unwrap()).unwrap();
    let mut prepared = super::spawn_container::PreparedChild::new_for_test(Some(child), None, None);
    prepared.swarm_reservation = Some(reservation);
    assert_eq!(context.summary().unwrap()["usage"], 2);
    prepared.rollback_once().await;
    assert_eq!(context.summary().unwrap()["usage"], 2);
    assert_eq!(context.summary().unwrap()["status"], "failed");
}

#[tokio::test]
async fn expired_budget_retains_coordinator_when_abort_endpoint_is_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 1);
    let mut child = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let summary = context.summary().unwrap();
    context
        .call(
            "_activate",
            json!([
                "parent",
                summary["members"][0]["reservation"],
                pid,
                process_start(pid),
                directory.path().join("missing.sock")
            ]),
        )
        .unwrap();
    context
        .call("stop", json!(["budget-exhausted", "deadline"]))
        .unwrap();
    let _ = super::swarm_lifecycle::settle(context).await;
    let exit = tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await;
    assert!(
        exit.is_err(),
        "coordinator must remain available for diagnostic reports"
    );
}

#[tokio::test]
async fn failed_run_cancels_coordinator_detached_jobs() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Arc::new(directory.path().to_path_buf());
    let tool = super::swarm_test_support::tool(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    );
    use crate::domain::swarm::CoordinationPort;
    let socket = directory.path().join("accept.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let coordinator = SwarmContext {
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
    };
    coordinator
        .register_endpoint(socket.to_str().unwrap())
        .unwrap();
    let start = tool.execute(r#"{"code":"import time, pathlib; pathlib.Path('writer-ready').touch()\nwhile not pathlib.Path('release-writer').exists(): time.sleep(0.01)\nopen('late-write','w').write('unsafe')","background":true}"#).await.unwrap();
    assert!(!start.is_error, "{}", start.content);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !directory.path().join("writer-ready").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let stop = tool
        .execute(r#"{"code":"from swarm import board; board.stop('failed','review regression')"}"#)
        .await
        .unwrap();
    assert!(!stop.is_error, "{}", stop.content);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "local suspension must not signal the coordinator through its socket"
    );
    std::fs::write(directory.path().join("release-writer"), "go").unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert!(
        !directory.path().join("late-write").exists(),
        "background writer survived terminal settlement"
    );
}

#[test]
fn abrupt_harness_death_retains_ownership_while_orphan_writer_survives() {
    use std::io::BufRead;
    let directory = tempfile::tempdir().unwrap();
    let parent = context(&directory);
    create(&parent, 2);
    let mut harness = std::process::Command::new("python3")
        .args(["-c", "import subprocess,time; p=subprocess.Popen(['sh','-c','while true; do echo writing >> orphan-write; sleep 0.05; done'], start_new_session=True, stdout=subprocess.DEVNULL); print(p.pid,flush=True); time.sleep(30)"])
        .current_dir(directory.path()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let mut line = String::new();
    std::io::BufReader::new(harness.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let writer_pid: u32 = line.trim().parse().unwrap();
    let writer_start = process_start(writer_pid).unwrap();
    let worker = SwarmContext {
        member: "worker".into(),
        ..parent.clone()
    };
    parent.call("_admit", json!(["worker", "r"])).unwrap();
    parent
        .call(
            "_activate",
            json!([
                "worker",
                "r",
                harness.id(),
                process_start(harness.id()),
                null
            ]),
        )
        .unwrap();
    let task = worker
        .call("task_create", json!(["t", "write file", ["pass"]]))
        .unwrap();
    let claim = worker.call("claim", json!([task["id"]])).unwrap();
    worker
        .call(
            "reserve",
            json!([task["id"], claim["token"], ["orphan-write"]]),
        )
        .unwrap();
    harness.kill().unwrap();
    harness.wait().unwrap();
    let snapshot = super::swarm_lifecycle::reconcile(&parent).unwrap();
    let surviving = !process_confirmed_dead(writer_pid, &writer_start);
    super::swarm::terminate_member(writer_pid);
    assert!(surviving);
    assert_eq!(
        snapshot["file_count"], 1,
        "harness death cannot prove its execution scope stopped"
    );
    assert_eq!(snapshot["usage"], 2);
    assert_eq!(snapshot["status"], "failed");
    assert!(parent.call("recover", json!([task["id"]])).is_err());
}

#[tokio::test]
async fn terminal_cancellation_closes_queued_and_future_background_launches() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Arc::new(directory.path().to_path_buf());
    let tool = super::swarm_test_support::tool(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    );
    let context = SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
    };
    let started = tool
        .execute(r#"{"background":true,"code":"open('must-not-start','w').write('unsafe')"}"#)
        .await
        .unwrap();
    assert!(!started.is_error, "{}", started.content);
    // Current-thread runtime: the queued background future has not been polled.
    super::swarm::cancel_context_jobs(&context);
    let later = tool
        .execute(r#"{"background":true,"code":"print('new work')"}"#)
        .await
        .unwrap();
    assert!(
        later.is_error,
        "closed registry admitted a new job: {}",
        later.content
    );
    let job: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let result = tool
                .execute(&json!({"op":"status","job_id":job["job_id"]}).to_string())
                .await
                .unwrap();
            let state: serde_json::Value = serde_json::from_str(&result.content).unwrap();
            if state["status"] == "cancelled" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!directory.path().join("must-not-start").exists());
}

#[tokio::test]
async fn terminal_notifications_do_not_queue_impossible_inbox_work() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 2);
    let socket = directory.path().join("worker.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    context.call("_admit", json!(["worker", "r"])).unwrap();
    context
        .call("_activate", json!(["worker", "r", 123, "identity", socket]))
        .unwrap();
    context
        .call("stop", json!(["blocked", "report partial result"]))
        .unwrap();
    let warnings = super::swarm_lifecycle::notify(&context).await;
    assert!(warnings.is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "an ended run queued another wake hint"
    );
    let summary = context.summary().unwrap();
    assert_eq!(
        (summary["status"].as_str(), summary["outcome"].as_str()),
        (Some("paused"), Some("blocked"))
    );
}

#[tokio::test]
async fn live_wake_hint_is_coalesced_and_carries_an_actionable_generation() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 2);
    let socket = directory.path().join("worker.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    context.call("_admit", json!(["worker", "r"])).unwrap();
    context
        .call("_activate", json!(["worker", "r", 123, "identity", socket]))
        .unwrap();
    context
        .call("task_create", json!(["work", "implement", ["pass"]]))
        .unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = tokio::io::split(stream);
        let mut read = tokio::io::BufReader::new(read);
        let bytes = quecto_line_io::read_frame(&mut read, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
            .await
            .unwrap()
            .unwrap();
        let command: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(command["type"], "swarm_control");
        assert_eq!(command["action"], "wake");
        assert!(command["generation"].as_u64().unwrap() > 0);
        let response = json!({"type":"response","id":command["id"],"success":true});
        quecto_line_io::write_frame(
            &mut write,
            response.to_string().as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
        listener
    });
    assert!(super::swarm_lifecycle::notify(&context).await.is_empty());
    let listener = server.await.unwrap();
    assert!(super::swarm_lifecycle::notify(&context).await.is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn watcher_cancels_foreground_after_cancelled_outcome() {
    foreground_terminal_watcher("cancelled").await;
}

#[tokio::test]
async fn watcher_cancels_foreground_after_successful_outcome() {
    foreground_terminal_watcher("succeeded").await;
}

async fn foreground_terminal_watcher(outcome: &str) {
    // The production watcher is process-scoped. Give each outcome its own
    // process, using the same test entry point and real lifecycle adapter.
    if std::env::var("SWARM_WATCHER_TEST").as_deref() != Ok(outcome) {
        let name = std::thread::current().name().unwrap().to_owned();
        let output = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture"])
            .env("SWARM_WATCHER_TEST", outcome)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use crate::domain::swarm::CoordinationPort;
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 1);
    let workspace = Arc::new(directory.path().to_path_buf());
    let tool = SwarmTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    )
    .with_context(Some(context.clone()));
    super::swarm_lifecycle::supervise(
        context.clone(),
        context.snapshot().unwrap(),
        super::swarm_bridge::Participation::none(),
    );
    let terminal = if outcome == "succeeded" {
        "board.evidence('tests','proof','R1','command',True); board.complete('R1')"
    } else {
        "board.stop('cancelled','operator request')"
    };
    let code = format!(
        "from swarm import board; import time; {terminal}; time.sleep(2); open('late-write','w').write('escaped')"
    );
    let result = tool
        .execute(&json!({"op":"run", "code":code, "timeout_seconds":10}).to_string())
        .await
        .unwrap();
    assert!(
        !workspace.join("late-write").exists(),
        "foreground interpreter survived terminal settlement"
    );
    // Completion ends the run as a resumable pause holding `succeeded`;
    // cancellation stays terminal (#1729).
    let summary = context.summary().unwrap();
    let expected = if outcome == "succeeded" {
        ("paused", Some("succeeded"))
    } else {
        (outcome, None)
    };
    assert_eq!(
        (
            summary["status"].as_str().unwrap(),
            summary["outcome"].as_str()
        ),
        expected
    );
    assert!(result.content.contains("cancelled"), "{}", result.content);
}

#[test]
fn pause_receipt_uses_control_generation_and_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 1);
    let paused = context.pause("await supervisor").unwrap();
    assert_eq!(paused["status"], "paused");
    let generation = paused["generation"]
        .as_u64()
        .expect("transaction control generation");
    assert!(generation > 0);
    assert_eq!(context.pause("same pause").unwrap(), paused);
    let refused = context.resume().expect_err("members cannot resume");
    assert!(
        refused.to_string().contains("outside the swarm"),
        "{refused}"
    );
    let resumed = context.resume_external().unwrap();
    assert_eq!(resumed["status"], "running");
    assert!(resumed["generation"].as_u64().unwrap() > generation);
    assert_eq!(context.resume_external().unwrap(), resumed);
}

#[tokio::test]
async fn summary_cursor_suppresses_unchanged_tool_payload_and_rejects_bad_cursors() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 1);
    let cursor = context.summary().unwrap()["event_cursor"].clone();
    let delta = super::swarm_control::control(context.clone(), "summary", json!({"since":cursor}))
        .await
        .unwrap();
    assert_eq!(delta["unchanged"], true);
    assert!(delta.to_string().len() < 150);
    for input in [json!({"since":-1}), json!({"since":"bad"})] {
        assert!(
            super::swarm_control::control(context.clone(), "summary", input)
                .await
                .is_err()
        );
    }
    for input in [
        json!({"limit":101}),
        json!({"limit":0}),
        json!({"after":-1}),
    ] {
        assert!(
            super::swarm_control::control(context.clone(), "events", input)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn resumed_swarm_can_start_python_jobs_after_suspension() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Arc::new(directory.path().to_path_buf());
    let tool = super::swarm_test_support::tool(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    );
    let context = SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
    };
    context.pause("inspect retained state").unwrap();
    super::swarm_lifecycle::settle(context.clone())
        .await
        .unwrap();
    context.resume_external().unwrap();
    let result = tool
        .execute(r#"{"code":"print('resumed')"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "resume permanently closed the execution registry: {}",
        result.content
    );
    assert!(result.content.contains("resumed"));
}

#[tokio::test]
async fn paused_summary_delta_is_read_only_and_stays_compact() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 1);
    context.pause("inspection").unwrap();
    let cursor = context.summary().unwrap()["event_cursor"].clone();
    let delta = super::swarm_control::control(context.clone(), "summary", json!({"since":cursor}))
        .await
        .unwrap();
    assert_eq!(delta["unchanged"], true);
    assert!(delta.to_string().len() < 150);
}

#[path = "swarm_bridge_admission_tests.rs"]
mod admission_tests;
