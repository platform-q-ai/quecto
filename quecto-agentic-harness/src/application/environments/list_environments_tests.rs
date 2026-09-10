use super::ListEnvironmentsQuery;
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

fn record(reference: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.to_string(),
        environment_id: format!("runtime-{reference}"),
        environment_uuid: format!("uuid-{reference}"),
        name: Some(format!("named-{reference}")),
        workspace_path: PathBuf::from(format!("/workspace/{reference}/with spaces")),
        repository: format!("https://example.test/{reference}.git"),
        script_name: format!("script-{reference}"),
        retained_exec_argv: vec!["exec".into(), reference.into()],
        retained_kill_argv: vec!["kill".into(), reference.into()],
        retained_cleanup_argv: vec!["cleanup".into(), reference.into()],
        retained_inspect_argv: vec!["inspect".into(), reference.into()],
        members: vec![
            format!("member-z-{reference}"),
            format!("member-a-{reference}"),
        ],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({"reference": reference, "nested": {"values": [1, "two"]}}),
        last_error: None,
    }
}

fn lifecycle_records() -> Vec<EnvironmentRecord> {
    let running = record("C1");
    let mut empty = record("C10");
    empty.members.clear();
    empty.name = None;
    let mut killing = record("C2");
    killing.status = EnvironmentStatus::Killing;
    let mut stopped = record("C3");
    stopped.status = EnvironmentStatus::Stopped;
    stopped.members.clear();
    let mut failed = record("C4");
    failed.status = EnvironmentStatus::CleanupFailed;
    failed.last_error = Some("cleanup exited with status 17".into());
    vec![running, empty, killing, stopped, failed]
}

#[test]
fn empty_registry_returns_empty_detached_snapshot() {
    let registry = EnvironmentRegistry::new();
    let query = ListEnvironmentsQuery::new(registry.clone());
    let snapshot = query.execute();
    assert!(snapshot.is_empty());
    registry.commit(record("C1"));
    assert!(snapshot.is_empty());
}

#[test]
fn preserves_registry_iteration_order_and_complete_records_in_every_lifecycle_state() {
    let registry = EnvironmentRegistry::new();
    let expected = lifecycle_records();
    for record in expected.iter().rev() {
        registry.commit(record.clone());
    }
    let snapshot = ListEnvironmentsQuery::new(registry).execute();
    // Preserve existing lexical iteration (C10 before C2), not numeric ref order.
    assert_eq!(snapshot, expected);
    assert_eq!(
        snapshot
            .iter()
            .map(EnvironmentRecord::status_label)
            .collect::<Vec<_>>(),
        ["running", "empty", "killing", "stopped", "cleanup-failed"]
    );
}

#[test]
fn populated_snapshot_is_detached_from_later_registry_mutations() {
    let registry = EnvironmentRegistry::new();
    let expected = lifecycle_records();
    for record in &expected {
        registry.commit(record.clone());
    }
    let query = ListEnvironmentsQuery::new(registry.clone());
    let snapshot = query.execute();
    registry.add_member("C1", "later-member").unwrap();
    let inspect = registry.begin_inspect("C1", "later-member").unwrap();
    registry.record_inspect_success(inspect, serde_json::json!({"nested": {"changed": true}}));
    let kill = registry.begin_kill("C1").unwrap();
    registry.fail_kill(kill, "later cleanup error");
    assert_eq!(registry.remove("C3"), Some(expected[3].clone()));
    registry.commit(record("C5"));

    assert_eq!(snapshot, expected);
    let current = query.execute();
    let mut changed = expected[0].clone();
    changed.members.push("later-member".into());
    changed.metadata["nested"] = serde_json::json!({"changed": true});
    changed.status = EnvironmentStatus::CleanupFailed;
    changed.last_error = Some("later cleanup error".into());
    assert_eq!(
        current,
        vec![
            changed,
            expected[1].clone(),
            expected[2].clone(),
            expected[4].clone(),
            record("C5")
        ]
    );
}

#[test]
fn concurrent_readers_observe_whole_duplicate_free_ordered_snapshots() {
    let registry = EnvironmentRegistry::new();
    let baseline = lifecycle_records();
    for record in &baseline {
        registry.commit(record.clone());
    }
    // Only C1 is replaced atomically: each observed record must be one complete
    // committed version, while the entire stable inventory must remain present.
    let mut replacement = baseline[0].clone();
    replacement.status = EnvironmentStatus::CleanupFailed;
    replacement.members = vec!["replacement-member".into()];
    replacement.metadata = serde_json::json!({"generation": "replacement"});
    replacement.last_error = Some("replacement error".into());
    let barrier = Arc::new(Barrier::new(4));
    std::thread::scope(|scope| {
        let writer_registry = registry.clone();
        let writer_barrier = barrier.clone();
        let original = baseline[0].clone();
        let replacement = &replacement;
        scope.spawn(move || {
            for iteration in 0..128 {
                writer_barrier.wait();
                writer_registry.commit(if iteration % 2 == 0 {
                    replacement.clone()
                } else {
                    original.clone()
                });
                writer_barrier.wait();
            }
        });
        for _ in 0..3 {
            let query = ListEnvironmentsQuery::new(registry.clone());
            let barrier = barrier.clone();
            let baseline = &baseline;
            scope.spawn(move || {
                let mut snapshots = Vec::new();
                for _ in 0..128 {
                    barrier.wait();
                    let snapshot = query.execute();
                    barrier.wait();
                    snapshots.push(snapshot);
                }
                // Assert after all rounds so failures cannot strand peers at a barrier.
                for snapshot in snapshots {
                    assert_eq!(snapshot.len(), baseline.len());
                    let refs = snapshot
                        .iter()
                        .map(|record| record.environment_ref.as_str())
                        .collect::<Vec<_>>();
                    assert_eq!(
                        refs.iter().copied().collect::<BTreeSet<_>>().len(),
                        snapshot.len()
                    );
                    assert_eq!(refs, ["C1", "C10", "C2", "C3", "C4"]);
                    for (actual, original) in snapshot.iter().zip(baseline) {
                        assert!(
                            actual == original
                                || (actual.environment_ref == "C1" && actual == replacement)
                        );
                        assert!(matches!(
                            actual.status,
                            EnvironmentStatus::Running
                                | EnvironmentStatus::Killing
                                | EnvironmentStatus::Stopped
                                | EnvironmentStatus::CleanupFailed
                        ));
                        assert!(
                            ["running", "empty", "killing", "stopped", "cleanup-failed"]
                                .contains(&actual.status_label())
                        );
                    }
                }
            });
        }
    });
}
