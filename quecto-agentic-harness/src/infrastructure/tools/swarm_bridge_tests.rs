use super::swarm::SwarmTool;
use super::swarm_bridge::{SwarmContext, process_confirmed_dead, process_start};
use crate::application::tools::ports::Tool;
use serde_json::json;
use std::sync::Arc;

fn context(directory: &tempfile::TempDir) -> SwarmContext {
    SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
        checkout: directory.path().to_path_buf(),
        member: "parent".into(),
    }
}

fn create(context: &SwarmContext, limit: u64) {
    std::fs::create_dir_all(context.checkout.join(".quecto")).unwrap();
    super::call_work::off_the_runtime(|| {
        context.call("create", json!(["ship", [], [{"id":"tests","kind":"command","description":"pass"}], limit,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 300]))
    })
    .unwrap();
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
async fn host_tool_rejects_every_op_even_with_a_planted_store() {
    let directory = tempfile::tempdir().unwrap();
    create(&context(&directory), 1);
    let tool = SwarmTool::new();
    for request in [r#"{"op":"summary"}"#, r#"{"op":"claim","task_id":1}"#] {
        let result = tool.execute(request).await.unwrap();
        assert!(result.is_error, "{request}");
        assert!(result.content.contains("container-only"), "{request}");
    }
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
    futures::executor::block_on(reservation.launched(std::process::id())).unwrap();
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

#[tokio::test]
async fn run_creation_requires_authorized_container_and_bounded_policy() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let tool = SwarmTool::new().with_context(Some(context.clone()));
    let invalid_deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let invalid = tool
        .execute(&json!({"op":"create","goal":"ship","constraints":[],"criteria":[{"id":"test","kind":"command","description":"pass"}],"member_limit":26,"deadline":invalid_deadline}).to_string())
        .await
        .unwrap();
    assert!(invalid.is_error);
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let created = tool.execute(&json!({"op":"create","goal":"ship","constraints":[],"criteria":[{"id":"test","kind":"command","description":"pass"}],"member_limit":25,"deadline":deadline}).to_string()).await.unwrap();
    assert!(!created.is_error, "{}", created.content);
    assert_eq!(
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.summary()).unwrap()["usage"],
        1
    );
    for index in 1..25 {
        crate::infrastructure::tools::call_work::off_the_runtime(|| {
            context.call(
                "_admit",
                json!([format!("worker-{index}"), format!("r-{index}")]),
            )
        })
        .unwrap();
    }
    assert_eq!(
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.summary()).unwrap()["usage"],
        25
    );
    assert!(
        crate::infrastructure::tools::call_work::off_the_runtime(
            || context.call("_admit", json!(["worker-25", "r-25"]))
        )
        .unwrap_err()
        .to_string()
        .contains("reuse")
    );
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
    let summary =
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.summary()).unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call(
            "_activate",
            json!([
                "parent",
                summary["members"][0]["reservation"],
                pid,
                process_start(pid),
                directory.path().join("missing.sock")
            ]),
        )
    })
    .unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("stop", json!(["budget-exhausted", "deadline"]))
    })
    .unwrap();
    let _ = super::swarm_lifecycle::settle(context).await;
    let exit = tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await;
    assert!(
        exit.is_err(),
        "coordinator must remain available for diagnostic reports"
    );
}

#[tokio::test]
async fn terminal_notifications_do_not_queue_impossible_inbox_work() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 2);
    let socket = directory.path().join("worker.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("_admit", json!(["worker", "r"]))
    })
    .unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("_activate", json!(["worker", "r", 123, "identity", socket]))
    })
    .unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("stop", json!(["blocked", "report partial result"]))
    })
    .unwrap();
    let warnings = super::swarm_lifecycle::notify(&context).await;
    assert!(warnings.is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "an ended run queued another wake hint"
    );
    let summary =
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.summary()).unwrap();
    assert_eq!(
        (summary["status"].as_str(), summary["outcome"].as_str()),
        (Some("paused"), Some("blocked"))
    );
}

/// A failed run's settlement suspends the coordinator's own inference
/// locally: it never signals the coordinator through its socket.
#[tokio::test]
async fn a_failed_run_never_signals_its_coordinator_through_its_socket() {
    use crate::application::swarm::ports::CoordinationPort;
    let directory = tempfile::tempdir().unwrap();
    let tool = super::swarm_test_support::tool(
        Arc::new(directory.path().to_path_buf()),
        crate::composition::swarm::swarm_board(),
    );
    let socket = directory.path().join("accept.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let coordinator = SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
    };
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        coordinator.register_endpoint(socket.to_str().unwrap())
    })
    .unwrap();
    let stop = tool
        .execute(r#"{"op":"stop","status":"failed","reason":"review regression"}"#)
        .await
        .unwrap();
    assert!(!stop.is_error, "{}", stop.content);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "local suspension must not signal the coordinator through its socket"
    );
}

#[tokio::test]
async fn live_wake_hint_is_coalesced_and_carries_an_actionable_generation() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 2);
    let socket = directory.path().join("worker.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("_admit", json!(["worker", "r"]))
    })
    .unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("_activate", json!(["worker", "r", 123, "identity", socket]))
    })
    .unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call("task_create", json!(["work", "implement", ["pass"]]))
    })
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
    let cursor = crate::infrastructure::tools::call_work::off_the_runtime(|| context.summary())
        .unwrap()["event_cursor"]
        .clone();
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
async fn paused_summary_delta_is_read_only_and_stays_compact() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 1);
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.pause("inspection"))
        .unwrap();
    let cursor = crate::infrastructure::tools::call_work::off_the_runtime(|| context.summary())
        .unwrap()["event_cursor"]
        .clone();
    let delta = super::swarm_control::control(context.clone(), "summary", json!({"since":cursor}))
        .await
        .unwrap();
    assert_eq!(delta["unchanged"], true);
    assert!(delta.to_string().len() < 150);
}

#[path = "swarm_bridge_admission_tests.rs"]
mod admission_tests;
#[path = "swarm_bridge_loss_tests.rs"]
mod loss_tests;

/// A board refusal reaches the tool boundary as it always has (#2278):
/// `swarm: ` and the refusal's text as a JSON string, quotes included.
#[test]
fn board_errors_keep_the_swarm_quoted_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    create(&context, 1);
    let refused = context.call("claim", json!([999])).unwrap_err();
    assert!(
        matches!(&refused, crate::domain::error::DomainError::Tool(text) if text == "swarm: \"unknown task\""),
        "{refused:?}"
    );
}

#[path = "swarm_bridge_host_tests.rs"]
mod host_tests;
