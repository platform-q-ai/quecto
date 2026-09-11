//! #1925: provenance of merged descendants.
//!
//! Root R launches local child A; A launches B and reports it in its
//! `subagent_state_changed` snapshot. If A dies WITHOUT receiving SIGTERM
//! (SIGKILL/OOM/panic) A never tears B down, so R's reaper cascade must be
//! able to — but only when B provably lives in R's pid namespace. A B reported
//! from inside a container carries a foreign pid and must never be signalled.

use super::*;
use crate::infrastructure::tools::process_ownership::{Lease, ProcessOwnership};
use crate::infrastructure::tools::spawn_reaper::{ReaperContext, spawn_reaper_task};
use crate::infrastructure::tools::subagent_environment_wire::BACKEND_LOCAL;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_exit_signal_channel, new_registry,
};
use std::time::Duration;

fn spawn_sleep() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn sleep")
}

fn snapshot_from_a(b_pid: u32, backend: Option<&str>) -> serde_json::Value {
    let mut b = serde_json::json!({
        "agentUuid": "b-uuid", "agentId": "B", "parentId": "a-uuid",
        "status": "running", "pid": b_pid, "socketPath": "/tmp/b.sock"
    });
    if let Some(backend) = backend {
        b["executionBackend"] = serde_json::Value::String(backend.into());
    }
    serde_json::json!({"type": "subagent_state_changed", "subagents": [b]})
}

/// Register A as a locally launched child whose real process is `a_child`
/// (a `true` that exits 0: reaped without any SIGTERM), then merge A's
/// snapshot of B. Returns after A's reaper has run its cascade.
async fn launch_a_merge_b_and_let_a_die(
    a_entry: SubagentEntry,
    snapshot: serde_json::Value,
) -> crate::infrastructure::tools::subagent_registry::SubagentRegistry {
    let registry = new_registry();
    let a_child = tokio::process::Command::new("true").spawn().unwrap();
    let mut a_entry = a_entry;
    a_entry.process_ownership = ProcessOwnership::launched(&a_child);
    let ownership = a_entry.process_ownership.clone();
    registry.lock().unwrap().insert("a-uuid".into(), a_entry);
    assert!(merge_and_forward_state_changed(&snapshot, &registry, "a-uuid").is_some());

    let (exit_tx, mut exit_rx) = new_exit_signal_channel();
    spawn_reaper_task(
        a_child,
        registry.clone(),
        "a-uuid".into(),
        exit_tx,
        None,
        ReaperContext {
            ownership,
            swarm_context: None,
        },
    );
    tokio::time::timeout(Duration::from_secs(5), exit_rx.changed())
        .await
        .unwrap()
        .unwrap();
    // The cascade runs after the exit signal is published; wait until it has
    // marked B dead (removed entries stay as Exited rows).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while registry.lock().unwrap()["b-uuid"].status != SubagentStatus::Exited {
        assert!(
            std::time::Instant::now() < deadline,
            "cascade never removed B"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    registry
}

fn launched_a() -> SubagentEntry {
    let mut a = SubagentEntry::new("/tmp/a.sock".into(), 4141);
    a.process_ownership = ProcessOwnership::launched_for_test();
    a
}

#[test]
fn host_local_descendant_of_launched_child_gets_same_namespace_lease() {
    let registry = new_registry();
    let a = launched_a();
    registry.lock().unwrap().insert("a-uuid".into(), a);

    merge_and_forward_state_changed(
        &snapshot_from_a(4242, Some(BACKEND_LOCAL)),
        &registry,
        "a-uuid",
    );
    let guard = registry.lock().unwrap();
    assert_eq!(
        guard["b-uuid"].process_ownership.lease(),
        Lease::Reported {
            same_namespace: true
        }
    );
    // A's own launched lease is untouched by the merge.
    assert_eq!(guard["a-uuid"].process_ownership.lease(), Lease::Launched);
}

#[test]
fn descendants_without_a_same_namespace_proof_stay_unsignallable() {
    // (1) Forwarding child sits behind an environment boundary (container).
    let registry = new_registry();
    let mut a = launched_a();
    a.environment_ref = Some("env-1".into());
    registry.lock().unwrap().insert("a-uuid".into(), a);
    merge_and_forward_state_changed(
        &snapshot_from_a(4242, Some(BACKEND_LOCAL)),
        &registry,
        "a-uuid",
    );
    assert_eq!(
        registry.lock().unwrap()["b-uuid"].process_ownership.lease(),
        Lease::Reported {
            same_namespace: false
        }
    );

    // (2) Forwarding child is itself unowned (fixture / script backend).
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), SubagentEntry::new("/tmp/a.sock".into(), 0));
    merge_and_forward_state_changed(
        &snapshot_from_a(4242, Some(BACKEND_LOCAL)),
        &registry,
        "a-uuid",
    );
    assert!(
        !registry.lock().unwrap()["b-uuid"]
            .process_ownership
            .is_owned()
    );

    // (3) Descendant does not report a local backend (legacy snapshot).
    let registry = new_registry();
    let a = launched_a();
    registry.lock().unwrap().insert("a-uuid".into(), a);
    merge_and_forward_state_changed(&snapshot_from_a(4242, None), &registry, "a-uuid");
    assert!(
        !registry.lock().unwrap()["b-uuid"]
            .process_ownership
            .is_owned()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_local_grandchild_is_terminated_when_its_parent_dies_without_sigterm() {
    let mut b_proc = spawn_sleep();
    let a = SubagentEntry::new("/tmp/a.sock".into(), 0);
    let _registry =
        launch_a_merge_b_and_let_a_die(a, snapshot_from_a(b_proc.id(), Some(BACKEND_LOCAL))).await;

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Ok(Some(status)) = b_proc.try_wait() {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            break None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let Some(status) = status else {
        let _ = b_proc.kill();
        let _ = b_proc.wait();
        panic!("host-local grandchild must be SIGTERMed by the reaper cascade");
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn container_reported_grandchild_is_never_signalled_by_the_cascade() {
    let mut b_proc = spawn_sleep();
    let mut a = SubagentEntry::new("/tmp/a.sock".into(), 0);
    a.environment_ref = Some("env-1".into());
    let _registry =
        launch_a_merge_b_and_let_a_die(a, snapshot_from_a(b_proc.id(), Some(BACKEND_LOCAL))).await;

    tokio::time::sleep(Duration::from_millis(150)).await;
    let still_running = matches!(b_proc.try_wait(), Ok(None));
    let _ = b_proc.kill();
    let _ = b_proc.wait();
    assert!(
        still_running,
        "a pid reported from another namespace must not be signalled"
    );
}

#[test]
fn descendant_pruned_by_a_later_snapshot_loses_its_lease() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), launched_a());
    merge_and_forward_state_changed(
        &snapshot_from_a(4242, Some(BACKEND_LOCAL)),
        &registry,
        "a-uuid",
    );
    assert!(
        registry.lock().unwrap()["b-uuid"]
            .process_ownership
            .is_owned()
    );

    // A's next snapshot omits B: A reaped it, and pid 4242 may be reused.
    let empty = serde_json::json!({"type": "subagent_state_changed", "subagents": []});
    merge_and_forward_state_changed(&empty, &registry, "a-uuid");
    let guard = registry.lock().unwrap();
    assert_eq!(guard["b-uuid"].status, SubagentStatus::Exited);
    assert!(
        !guard["b-uuid"].process_ownership.is_owned(),
        "a pruned descendant must not keep signal authority over a reusable pid"
    );
}
