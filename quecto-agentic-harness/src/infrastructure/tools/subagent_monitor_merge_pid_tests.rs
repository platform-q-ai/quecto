//! #1940: a merged descendant carries no process authority and no pid.
//!
//! Root R launches local child A; A launches B and reports it in its
//! `subagent_state_changed` snapshot. Whatever pid the snapshot names — R's
//! own, A's, pid 1 or 2 (init and the coordinator inside a container), a
//! reused pid, a pid from another namespace — the merged row records none
//! of it and no teardown path ever signals it: B ends through A's own
//! teardown and the parent-loss binding, never from R.

use super::*;
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
        "status": "running", "pid": b_pid, "socketPath": "/tmp/b.sock",
        "launchGeneration": 7
    });
    if let Some(backend) = backend {
        b["executionBackend"] = serde_json::Value::String(backend.into());
    }
    serde_json::json!({"type": "subagent_state_changed", "subagents": [b]})
}

/// Register A as a locally launched child whose real process is `a_child`
/// (a `true` that exits 0: reaped without any signal), then merge A's
/// snapshot of B. Returns after A's reaper has run its cascade.
async fn launch_a_merge_b_and_let_a_die(
    a_entry: SubagentEntry,
    snapshot: serde_json::Value,
) -> crate::infrastructure::tools::subagent_registry::SubagentRegistry {
    let registry = new_registry();
    let supervisor = std::sync::Arc::new(
        crate::infrastructure::processes::owned_child_supervisor::OwnedChildSupervisor::new(),
    );
    let handle = supervisor
        .spawn(
            tokio::process::Command::new("true"),
            crate::infrastructure::processes::owned_child_supervisor::ProcessGroup::Inherited,
        )
        .await
        .unwrap()
        .handle;
    let mut a_entry = a_entry;
    a_entry.owned_child = Some(handle);
    a_entry.owned_child_supervisor = Some(supervisor.clone());
    registry.lock().unwrap().insert("a-uuid".into(), a_entry);
    assert!(merge_and_forward_state_changed(&snapshot, &registry, "a-uuid").is_some());

    let (exit_tx, mut exit_rx) = new_exit_signal_channel();
    let observer =
        crate::infrastructure::tools::subagent_teardown_wiring::build_lifecycle_use_cases(
            registry.clone(),
            None,
            None,
        )
        .observe_exit;
    spawn_reaper_task(
        handle,
        supervisor,
        ReaperContext {
            exit_tx,
            child: crate::domain::subagent_teardown::DelegatedAgentIdentity::new(
                "a-uuid",
                crate::domain::subagent_teardown::LaunchGeneration::new(1),
            ),
            observer,
            swarm_member: None,
        },
    );
    tokio::time::timeout(Duration::from_secs(5), exit_rx.changed())
        .await
        .unwrap()
        .unwrap();
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

/// The pids an adversarial or merely confused snapshot could name: this
/// process, its parent, init, the in-container coordinator, a pid that was
/// just reaped (reusable) and an obviously foreign one.
fn adversarial_pids() -> Vec<(&'static str, u32)> {
    let mut reaped = std::process::Command::new("true").spawn().unwrap();
    let reaped_pid = reaped.id();
    reaped.wait().unwrap();
    // SAFETY: getppid takes no arguments and cannot fail.
    let parent = unsafe { libc::getppid() } as u32;
    vec![
        ("self", std::process::id()),
        ("parent", parent),
        ("init", 1),
        ("container coordinator", 2),
        ("reused", reaped_pid),
        ("foreign namespace", 4_000_000),
    ]
}

#[test]
fn merged_descendants_record_no_pid_whatever_the_snapshot_names() {
    for (label, pid) in adversarial_pids() {
        for backend in [Some(BACKEND_LOCAL), Some("container"), None] {
            let registry = new_registry();
            registry
                .lock()
                .unwrap()
                .insert("a-uuid".into(), SubagentEntry::new("/tmp/a.sock".into(), 0));
            let forwarded = merge_and_forward_state_changed(
                &snapshot_from_a(pid, backend),
                &registry,
                "a-uuid",
            )
            .expect("merged");
            let guard = registry.lock().unwrap();
            let b = &guard["b-uuid"];
            assert_eq!(
                b.pid, REPORTED_DESCENDANT_PID,
                "{label} pid {pid} ({backend:?}) must not be recorded"
            );
            assert!(b.owned_child.is_none() && !b.holds_owned_child());
            assert_eq!(
                b.reported_generation.map(|g| g.get()),
                Some(7),
                "routing identity is kept: the generation, never the pid"
            );
            // The re-broadcast carries no pid upward either.
            let event: serde_json::Value = serde_json::from_str(&forwarded).unwrap();
            let rows = event["subagents"].as_array().unwrap();
            let row = rows.iter().find(|r| r["agentUuid"] == "b-uuid").unwrap();
            assert_eq!(row["pid"], serde_json::json!(REPORTED_DESCENDANT_PID));
        }
    }
}

/// A later snapshot naming a new pid for the same row changes nothing: the
/// row never carries one.
#[test]
fn a_re_reported_pid_is_dropped_too() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), SubagentEntry::new("/tmp/a.sock".into(), 0));
    merge_and_forward_state_changed(
        &snapshot_from_a(1, Some(BACKEND_LOCAL)),
        &registry,
        "a-uuid",
    );
    merge_and_forward_state_changed(
        &snapshot_from_a(std::process::id(), Some(BACKEND_LOCAL)),
        &registry,
        "a-uuid",
    );
    assert_eq!(
        registry.lock().unwrap()["b-uuid"].pid,
        REPORTED_DESCENDANT_PID
    );
}

/// A reported descendant is never signalled from here, whether the
/// forwarding child ran on the host or inside an environment: B's process
/// (a real `sleep` whose pid A's snapshot names) survives A's death and
/// the cascade that removes B's row. B ends only when its own bound
/// control connection is lost (#1935), never by a signal from R.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_reported_descendant_pid_is_ever_signalled_by_the_cascade() {
    for environment_ref in [None, Some("env-1")] {
        let mut b_proc = spawn_sleep();
        let mut a = SubagentEntry::new("/tmp/a.sock".into(), 0);
        a.environment_ref = environment_ref.map(str::to_string);
        let registry =
            launch_a_merge_b_and_let_a_die(a, snapshot_from_a(b_proc.id(), Some(BACKEND_LOCAL)))
                .await;
        assert_eq!(
            registry.lock().unwrap()["b-uuid"].status,
            SubagentStatus::Exited,
            "B's row falls with A's subtree"
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        let still_running = matches!(b_proc.try_wait(), Ok(None));
        let _ = b_proc.kill();
        let _ = b_proc.wait();
        assert!(
            still_running,
            "a reported pid (environment {environment_ref:?}) must never be signalled"
        );
    }
}

/// The self pid is the sharpest probe: had any path signalled a merged
/// row's pid, this test process would have received it. The cascade runs
/// and this process is still here to assert.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_snapshot_naming_this_process_produces_no_effect_on_it() {
    let a = SubagentEntry::new("/tmp/a.sock".into(), 0);
    let registry =
        launch_a_merge_b_and_let_a_die(a, snapshot_from_a(std::process::id(), Some(BACKEND_LOCAL)))
            .await;
    let guard = registry.lock().unwrap();
    assert_eq!(guard["b-uuid"].status, SubagentStatus::Exited);
    assert_eq!(guard["b-uuid"].pid, REPORTED_DESCENDANT_PID);
}

/// A snapshot pruning B (A reaped it) compensates the row without touching
/// any process: the prune is the whole of B's compensation.
#[test]
fn descendant_pruned_by_a_later_snapshot_is_compensated_without_a_signal() {
    let mut b_proc = spawn_sleep();
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), SubagentEntry::new("/tmp/a.sock".into(), 0));
    merge_and_forward_state_changed(
        &snapshot_from_a(b_proc.id(), Some(BACKEND_LOCAL)),
        &registry,
        "a-uuid",
    );
    let empty = serde_json::json!({"type": "subagent_state_changed", "subagents": []});
    merge_and_forward_state_changed(&empty, &registry, "a-uuid");
    {
        let guard = registry.lock().unwrap();
        assert_eq!(guard["b-uuid"].status, SubagentStatus::Exited);
        assert_eq!(
            guard["b-uuid"].teardown_phase(),
            crate::infrastructure::tools::subagent_registry::TeardownPhase::Compensated
        );
    }
    std::thread::sleep(Duration::from_millis(100));
    let still_running = matches!(b_proc.try_wait(), Ok(None));
    let _ = b_proc.kill();
    let _ = b_proc.wait();
    assert!(still_running, "a pruned row's process is never signalled");
}

#[test]
fn merge_recovers_from_a_poisoned_registry_lock() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), SubagentEntry::new("/tmp/a.sock".into(), 0));
    let shared = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = shared.lock().unwrap();
        panic!("poison registry for coverage");
    })
    .join();
    assert!(registry.lock().is_err());
    merge_and_forward_state_changed(
        &snapshot_from_a(4242, Some(BACKEND_LOCAL)),
        &registry,
        "a-uuid",
    );
    let guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(guard["b-uuid"].pid, REPORTED_DESCENDANT_PID);
    assert_eq!(guard["b-uuid"].status, SubagentStatus::Running);
}
