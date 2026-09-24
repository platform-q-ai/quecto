//! Restore judged against the hosted swarm store (round 4 M1/L3, #2033):
//! a `running` record whose container is gone but whose checkout hosts an
//! unfinished run is what the finalizer would have retained had the
//! master lived to see the coordinator exit — it is relabelled `retained`
//! with the finalizer's reason, never `stopped`, so no collector ever
//! sees its board as removable; a record an older build relabelled
//! `stopped` at restore while it was retained is restored to `retained`.
use std::sync::{Arc, Mutex};

use super::restore_registry_tests::{FakeHosted, process, record, store_with};
use super::{RELABELLED_BY_OLDER_BUILD, RestoreRegistry, unfinished_run_reason};
use crate::application::environments::dto::EnvironmentLiveness;
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environment_registry::EnvironmentStatus;
use crate::domain::environment_registry::GONE_AT_RESTORE;
use crate::domain::environment_retention::{HostedSwarmRun, SwarmRunObservation};
use crate::domain::swarm::RunStatus;

fn run(id: &str, status: RunStatus, outcome: Option<RunStatus>) -> HostedSwarmRun {
    HostedSwarmRun {
        id: id.into(),
        status,
        outcome,
        coordinator: "coordinator".into(),
        deadline: 4_102_444_800.0,
    }
}

fn hosted(observations: Vec<(&str, SwarmRunObservation)>) -> Arc<FakeHosted> {
    Arc::new(FakeHosted {
        by_ref: Mutex::new(
            observations
                .into_iter()
                .map(|(r, o)| (r.to_string(), o))
                .collect(),
        ),
        asked: Mutex::new(vec![]),
    })
}

#[test]
fn a_running_record_gone_whose_checkout_hosts_an_unfinished_run_is_retained_not_stopped() {
    let store = store_with(vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Running),
    ]);
    let hosted = hosted(vec![
        (
            "C1",
            SwarmRunObservation::Run(run("run-1", RunStatus::Running, None)),
        ),
        (
            "C2",
            SwarmRunObservation::Run(run("run-2", RunStatus::Paused, None)),
        ),
    ]);
    let process = process(|_| EnvironmentLiveness::Gone);
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, hosted.clone()).execute("s");
    assert!(report.stopped.is_empty(), "{report:?}");
    let expected_c1 = unfinished_run_reason(&run("run-1", RunStatus::Running, None));
    assert_eq!(
        expected_c1,
        "run run-1 unfinished (running); container exited; environment retained for inspection, kill_container to remove"
    );
    assert_eq!(
        report.retained,
        [
            ("C1".to_string(), expected_c1.clone()),
            (
                "C2".to_string(),
                unfinished_run_reason(&run("run-2", RunStatus::Paused, None))
            )
        ]
    );
    for reference in ["C1", "C2"] {
        let seeded = registry.get(reference).unwrap();
        assert_eq!(seeded.status, EnvironmentStatus::Retained, "{seeded:?}");
        assert_eq!(seeded.last_error, None, "{seeded:?}");
        let on_file = store.load().unwrap();
        let on_file = on_file
            .iter()
            .find(|r| r.environment_ref == reference)
            .unwrap();
        assert_eq!(on_file.status, EnvironmentStatus::Retained);
        assert_eq!(on_file.metadata["retained"], seeded.metadata["retained"]);
    }
    assert_eq!(
        registry.get("C1").unwrap().metadata["retained"],
        serde_json::json!(expected_c1)
    );
    assert_eq!(
        store.corrections.lock().unwrap().as_slice(),
        ["C1", "C2"],
        "written conditionally, like any correction"
    );
    assert_eq!(hosted.asked.lock().unwrap().as_slice(), ["C1", "C2"]);
}

/// A run paused holding an outcome, or cancelled by its coordinator, is not
/// closed by its owner: the finalizer keeps that box (#2070), so restore
/// retains it too. Only a closed run, the placeholder or no store is stopped.
#[test]
fn a_running_record_gone_with_a_closed_run_a_placeholder_or_no_store_is_stopped() {
    let store = store_with(vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Running),
        record("C3", EnvironmentStatus::Running),
        record("C4", EnvironmentStatus::Running),
        record("C5", EnvironmentStatus::Running),
    ]);
    let placeholder = HostedSwarmRun {
        deadline: 0.0,
        ..run("run-3", RunStatus::Setup, None)
    };
    let hosted = hosted(vec![
        (
            "C1",
            SwarmRunObservation::Run(run("run-1", RunStatus::Paused, Some(RunStatus::Failed))),
        ),
        (
            "C2",
            SwarmRunObservation::Run(run("run-2", RunStatus::Succeeded, None)),
        ),
        ("C3", SwarmRunObservation::Run(placeholder)),
        ("C4", SwarmRunObservation::NoStore),
        (
            "C5",
            SwarmRunObservation::Run(run("run-5", RunStatus::Cancelled, None)),
        ),
    ]);
    let process = process(|_| EnvironmentLiveness::Gone);
    let (registry, report) = RestoreRegistry::new(store.clone(), process, hosted).execute("s");
    assert_eq!(report.stopped, ["C2", "C3", "C4"]);
    assert_eq!(
        report.retained,
        [
            (
                "C1".to_string(),
                unfinished_run_reason(&run("run-1", RunStatus::Paused, Some(RunStatus::Failed)))
            ),
            (
                "C5".to_string(),
                unfinished_run_reason(&run("run-5", RunStatus::Cancelled, None))
            )
        ]
    );
    for reference in ["C1", "C5"] {
        assert_eq!(
            registry.get(reference).unwrap().status,
            EnvironmentStatus::Retained
        );
    }
    for reference in ["C2", "C3", "C4"] {
        let seeded = registry.get(reference).unwrap();
        assert_eq!(seeded.status, EnvironmentStatus::Stopped, "{seeded:?}");
        assert_eq!(seeded.last_error.as_deref(), Some(GONE_AT_RESTORE));
        assert!(seeded.metadata.get("retained").is_none(), "{seeded:?}");
    }
}

#[test]
fn an_unreadable_hosted_store_retains_as_the_finalizer_would() {
    let store = store_with(vec![record("C1", EnvironmentStatus::Running)]);
    let hosted = hosted(vec![(
        "C1",
        SwarmRunObservation::Unreadable("database is locked".into()),
    )]);
    let process = process(|_| EnvironmentLiveness::Gone);
    let (registry, report) = RestoreRegistry::new(store, process, hosted).execute("s");
    let seeded = registry.get("C1").unwrap();
    assert_eq!(seeded.status, EnvironmentStatus::Retained);
    let reason = seeded.metadata["retained"].as_str().unwrap();
    assert!(
        reason.contains("could not be read (database is locked)")
            && reason.contains("container exited"),
        "{reason}"
    );
    assert_eq!(report.retained, [("C1".to_string(), reason.to_string())]);
}

#[test]
fn the_hosted_store_is_asked_only_of_a_running_record_whose_container_is_gone() {
    let store = store_with(vec![
        record("C1", EnvironmentStatus::Running),
        record("C2", EnvironmentStatus::Retained),
        record("C3", EnvironmentStatus::CleanupFailed),
        record("C4", EnvironmentStatus::Running),
        record("C5", EnvironmentStatus::Stopped),
    ]);
    // Every answer is an unfinished run: only C4 may act on it.
    let hosted = hosted(
        ["C1", "C2", "C3", "C4", "C5"]
            .into_iter()
            .map(|r| {
                (
                    r,
                    SwarmRunObservation::Run(run("run", RunStatus::Running, None)),
                )
            })
            .collect(),
    );
    let process = process(|record| match record.environment_ref.as_str() {
        "C1" => EnvironmentLiveness::Running,
        _ => EnvironmentLiveness::Gone,
    });
    let (registry, report) = RestoreRegistry::new(store, process, hosted.clone()).execute("s");
    assert_eq!(hosted.asked.lock().unwrap().as_slice(), ["C4"]);
    assert_eq!(report.restored, ["C1"]);
    // A failed explicit kill was the operator's intent: gone means stopped;
    // the collector still keeps its directory while the run is unfinished.
    assert_eq!(report.stopped, ["C3"]);
    assert_eq!(
        registry.get("C4").unwrap().status,
        EnvironmentStatus::Retained
    );
    assert_eq!(
        registry.get("C2").unwrap().status,
        EnvironmentStatus::Retained
    );
}

#[test]
fn an_observing_restore_seeds_the_retained_relabel_without_writing_it() {
    let store = store_with(vec![record("C1", EnvironmentStatus::Running)]);
    let hosted = hosted(vec![(
        "C1",
        SwarmRunObservation::Run(run("run-1", RunStatus::Running, None)),
    )]);
    let process = process(|_| EnvironmentLiveness::Gone);
    let (registry, report) = RestoreRegistry::new(store.clone(), process, hosted).observe("cli");
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Retained
    );
    assert!(store.corrections.lock().unwrap().is_empty());
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Running);
    assert_eq!(report.retained.len(), 1);
    assert!(
        report.diagnostics[0].starts_with("C1 would be recorded retained (run run-1 unfinished"),
        "{:?}",
        report.diagnostics
    );
}

// ─── L3: an older build's relabel of a retained record is undone ────────────

fn relabelled_by_older_build(
    reference: &str,
) -> crate::domain::environment_registry::EnvironmentRecord {
    let mut stale = record(reference, EnvironmentStatus::Stopped);
    stale.metadata = serde_json::json!({"retained": "run ended: blocked; environment retained for inspection, kill_container to remove"});
    stale.last_error = Some(GONE_AT_RESTORE.into());
    stale
}

#[test]
fn a_retained_record_an_older_build_relabelled_stopped_is_restored_retained() {
    let mut killed = record("C2", EnvironmentStatus::Stopped);
    killed.metadata = serde_json::json!({"retained": "run ended: blocked; environment retained for inspection, kill_container to remove"});
    // An explicit kill clears the last error: this one was ended on purpose.
    killed.last_error = None;
    let mut plain_gone = record("C3", EnvironmentStatus::Stopped);
    plain_gone.last_error = Some(GONE_AT_RESTORE.into());
    let store = store_with(vec![relabelled_by_older_build("C1"), killed, plain_gone]);
    let hosted = hosted(vec![]);
    let process = process(|_| panic!("a stopped record is not inspected"));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, hosted.clone()).execute("s");
    assert!(
        hosted.asked.lock().unwrap().is_empty(),
        "no store read either"
    );
    let c1 = registry.get("C1").unwrap();
    assert_eq!(c1.status, EnvironmentStatus::Retained, "{c1:?}");
    assert_eq!(c1.last_error, None, "the older build's reason is withdrawn");
    assert!(
        c1.metadata["retained"]
            .as_str()
            .unwrap()
            .starts_with("run ended: blocked"),
        "its own reason stands: {c1:?}"
    );
    assert_eq!(
        report.retained,
        [("C1".to_string(), RELABELLED_BY_OLDER_BUILD.to_string())]
    );
    assert_eq!(store.corrections.lock().unwrap().as_slice(), ["C1"]);
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Retained);
    // Half the signature is no signature.
    assert_eq!(
        registry.get("C2").unwrap().status,
        EnvironmentStatus::Stopped
    );
    assert_eq!(
        registry.get("C3").unwrap().status,
        EnvironmentStatus::Stopped
    );
    assert!(report.stopped.is_empty(), "{report:?}");
}

#[test]
fn the_older_build_relabel_is_undone_in_memory_only_by_an_observing_restore() {
    let store = store_with(vec![relabelled_by_older_build("C1")]);
    let process = process(|_| panic!("a stopped record is not inspected"));
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, hosted(vec![])).observe("cli");
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Retained
    );
    assert_eq!(store.load().unwrap()[0].status, EnvironmentStatus::Stopped);
    assert!(store.corrections.lock().unwrap().is_empty());
    assert!(
        report.diagnostics[0].contains("would be recorded retained"),
        "{:?}",
        report.diagnostics
    );
}

/// Round 5 (#2033): a retained environment whose *kill failed* also goes
/// gone → stopped, but it keeps its own kill error behind the restore note,
/// so it never wears the "relabelled by an older build" signature and the
/// operator's diagnosis survives.
#[test]
fn a_failed_retained_kill_that_went_gone_keeps_its_error_and_is_not_resurrected() {
    let mut failed = record("C1", EnvironmentStatus::CleanupFailed);
    failed.metadata = serde_json::json!({"retained": "run ended: blocked; environment retained for inspection, kill_container to remove"});
    failed.last_error = Some("retained kill refused: scripts/kill.sh differs".into());
    let store = store_with(vec![failed]);
    let process = process(|_| EnvironmentLiveness::Gone);
    let (registry, report) =
        RestoreRegistry::new(store.clone(), process, hosted(vec![])).execute("s");
    assert_eq!(report.stopped, ["C1"]);
    let c1 = registry.get("C1").unwrap();
    assert_eq!(c1.status, EnvironmentStatus::Stopped);
    let error = c1.last_error.clone().unwrap();
    assert!(error.starts_with(GONE_AT_RESTORE), "{error}");
    assert!(error.contains("earlier: retained kill refused"), "{error}");
    // A second restore sees no older-build signature: it stays stopped.
    let (registry, report) = RestoreRegistry::new(
        store.clone(),
        self::process(|_| panic!("a stopped record is not inspected")),
        hosted(vec![]),
    )
    .execute("s");
    assert_eq!(
        registry.get("C1").unwrap().status,
        EnvironmentStatus::Stopped
    );
    assert!(report.retained.is_empty(), "{report:?}");
}
