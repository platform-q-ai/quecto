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
    // A locally launched child: owned through the supervisor (#1935), no lease.
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

/// #1936: the cascade never signals a reported descendant's pid, whatever
/// namespace it reports from. A grandchild ends because its own parent's
/// bound control connection is lost (#1935), never by a signal from here.
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

/// Reviewer scenario: R launches host-local A; A launches container C
/// (script backend, environment); C launches D inside the container, which
/// reports `executionBackend: local` RELATIVE TO C with a container pid (2).
/// A forwards [C, D] to R. D must not be granted a lease in R's namespace.
#[test]
fn descendant_behind_a_container_hop_is_denied_even_when_it_reports_local() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), launched_a());
    let snapshot = serde_json::json!({"type": "subagent_state_changed", "subagents": [
        {"agentUuid": "c-uuid", "agentId": "C", "parentId": "a-uuid", "status": "running",
         "pid": 0, "executionBackend": "script",
         "environment": {"environmentRef": "env-c", "socketMode": "proxy"}},
        {"agentUuid": "d-uuid", "agentId": "D", "parentId": "c-uuid", "status": "running",
         "pid": 2, "executionBackend": "local", "socketPath": "/tmp/d.sock"}
    ]});
    merge_and_forward_state_changed(&snapshot, &registry, "a-uuid");
    let guard = registry.lock().unwrap();
    assert_eq!(
        guard["d-uuid"].process_ownership.lease(),
        Lease::Reported {
            same_namespace: false
        },
        "a pid reported from inside a container hop must never be signallable"
    );
    assert!(!guard["c-uuid"].process_ownership.is_owned());
}

/// Three all-local hops below a launched child still grant: A -> B -> C -> D,
/// every hop a host-local launch with an explicit parentId in the snapshot.
#[test]
fn all_local_multi_hop_chain_grants_same_namespace_lease() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), launched_a());
    let local = |uuid: &str, parent: &str, pid: u32| {
        serde_json::json!({"agentUuid": uuid, "agentId": uuid, "parentId": parent,
            "status": "running", "pid": pid, "executionBackend": "local",
            "socketPath": format!("/tmp/{uuid}.sock")})
    };
    // Deliberately out of order: D listed before its ancestors.
    let snapshot = serde_json::json!({"type": "subagent_state_changed", "subagents": [
        local("d-uuid", "c-uuid", 4004),
        local("b-uuid", "a-uuid", 4002),
        local("c-uuid", "b-uuid", 4003),
    ]});
    merge_and_forward_state_changed(&snapshot, &registry, "a-uuid");
    let guard = registry.lock().unwrap();
    for key in ["b-uuid", "c-uuid", "d-uuid"] {
        assert!(
            guard[key].process_ownership.is_owned(),
            "{key} sits below an all-local chain and must be signallable"
        );
    }
}

#[test]
fn chain_with_missing_or_unknown_parent_is_denied() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), launched_a());
    let snapshot = serde_json::json!({"type": "subagent_state_changed", "subagents": [
        {"agentUuid": "orphan", "agentId": "orphan", "status": "running", "pid": 4005,
         "executionBackend": "local"},
        {"agentUuid": "stranger", "agentId": "stranger", "parentId": "not-in-snapshot",
         "status": "running", "pid": 4006, "executionBackend": "local"}
    ]});
    merge_and_forward_state_changed(&snapshot, &registry, "a-uuid");
    let guard = registry.lock().unwrap();
    assert!(!guard["orphan"].process_ownership.is_owned());
    assert!(!guard["stranger"].process_ownership.is_owned());
}

/// Prune retirement must reach a cascade-removed clone that shares the lease,
/// not just the registry row.
#[test]
fn prune_retires_the_shared_lease_so_removed_clones_cannot_signal() {
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
    let removed_clone = registry.lock().unwrap()["b-uuid"].clone();
    assert!(removed_clone.process_ownership.is_owned());

    let empty = serde_json::json!({"type": "subagent_state_changed", "subagents": []});
    merge_and_forward_state_changed(&empty, &registry, "a-uuid");

    assert!(
        !removed_clone.process_ownership.is_owned(),
        "the clone shares the lease and must be retired with the row"
    );
    crate::infrastructure::tools::process_tree::SIGNAL_LOG
        .with(|log| *log.borrow_mut() = Some(Vec::new()));
    let signalled =
        crate::infrastructure::tools::subagent_cascade::terminate_removed_entry(&removed_clone);
    let log = crate::infrastructure::tools::process_tree::SIGNAL_LOG
        .with(|log| log.borrow_mut().take().unwrap());
    assert!(!signalled);
    assert!(log.is_empty(), "retired clone dispatched: {log:?}");
}

/// Legacy snapshots key descendants by `agentId` only (or carry an empty
/// `agentUuid`); the chain walk resolves those keys the same way the upsert
/// does, so a legacy all-local chain still grants.
#[test]
fn legacy_agent_id_keys_resolve_in_the_chain_walk() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), launched_a());
    let snapshot = serde_json::json!({"type": "subagent_state_changed", "subagents": [
        {"agentUuid": "", "agentId": "legacy-b", "parentId": "a-uuid", "status": "running",
         "pid": 4100, "executionBackend": "local"},
        {"agentId": "legacy-c", "parentId": "legacy-b", "status": "running",
         "pid": 4101, "executionBackend": "local"},
        // No key at all: upserted under a minted uuid, never signallable.
        {"displayName": "nameless", "parentId": "a-uuid", "status": "running",
         "pid": 4102, "executionBackend": "local"}
    ]});
    merge_and_forward_state_changed(&snapshot, &registry, "a-uuid");
    let guard = registry.lock().unwrap();
    assert!(guard["legacy-b"].process_ownership.is_owned());
    assert!(guard["legacy-c"].process_ownership.is_owned());
    let nameless = guard
        .values()
        .find(|e| e.display_name == "nameless")
        .expect("nameless descendant upserted under a minted key");
    assert!(!nameless.process_ownership.is_owned());
}

#[test]
fn merge_recovers_from_a_poisoned_registry_lock() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("a-uuid".into(), launched_a());
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
    assert!(guard["b-uuid"].process_ownership.is_owned());
}
