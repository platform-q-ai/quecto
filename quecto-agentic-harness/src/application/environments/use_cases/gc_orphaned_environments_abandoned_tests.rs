//! Abandoned runs (#2070): the collector keeps what the finalizer keeps — a
//! created run its owner has not closed, whatever its status — and collects
//! a directory nobody records that still hosts such a run only when asked
//! (`--abandoned`, or `--abandoned-after` once the directory is old enough),
//! saying why for each.
use super::super::dto::{AbandonedRuns, EnvironmentStateDir, GcRemoval, GcRequest};
use super::gc_orphaned_environments_hosted_tests::{kept_reason, removable, run};
use super::gc_orphaned_environments_tests::{Rig, dir, record};
use crate::domain::environment_registry::EnvironmentStatus;
use crate::domain::swarm::RunStatus;

fn request(abandoned: AbandonedRuns) -> GcRequest {
    GcRequest {
        dry_run: true,
        config: None,
        abandoned,
    }
}

/// A `stopped` record's directory hosting a run paused holding an outcome,
/// or cancelled by its coordinator, is kept: the finalizer keeps that box
/// (only the owner's close ends a run), and the collector must agree.
#[test]
fn the_collector_keeps_what_the_finalizer_keeps() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C2", "env-held", EnvironmentStatus::Stopped));
    rig.registry
        .commit(record("C3", "env-cancelled", EnvironmentStatus::Stopped));
    rig.registry
        .commit(record("C4", "env-closed", EnvironmentStatus::Stopped));
    *rig.host.dirs.lock().unwrap() = vec![
        dir("env-held", Some("quecto-env-held")),
        dir("env-cancelled", Some("quecto-env-cancelled")),
        dir("env-closed", Some("quecto-env-closed")),
    ];
    *rig.hosted.by_id.lock().unwrap() = vec![
        (
            "env-held".into(),
            run("run-h", RunStatus::Paused, Some(RunStatus::Failed)),
        ),
        (
            "env-cancelled".into(),
            run("run-c", RunStatus::Cancelled, None),
        ),
        (
            "env-closed".into(),
            run("run-k", RunStatus::Succeeded, None),
        ),
    ];
    let report = rig
        .use_case()
        .execute(&request(AbandonedRuns::Keep))
        .unwrap();
    assert!(
        kept_reason(&report, "env-held").contains("hosts swarm run run-h (paused holding failed)"),
        "{report:?}"
    );
    assert!(
        kept_reason(&report, "env-cancelled").contains("hosts swarm run run-c (cancelled)"),
        "{report:?}"
    );
    assert_eq!(
        removable(&report, "env-closed"),
        &GcRemoval::RetainedCleanup {
            environment_ref: "C4".into()
        }
    );
}

#[test]
fn an_unrecorded_directory_hosting_a_run_is_collected_only_when_asked() {
    let dirs = || {
        vec![
            EnvironmentStateDir {
                age_secs: Some(3_600),
                ..dir("env-hour", Some("quecto-env-hour"))
            },
            EnvironmentStateDir {
                age_secs: Some(3 * 86_400),
                ..dir("env-days", Some("quecto-env-days"))
            },
            EnvironmentStateDir {
                age_secs: None,
                ..dir("env-ageless", Some("quecto-env-ageless"))
            },
        ]
    };
    let hosted = || {
        vec![
            ("/s/env-hour".into(), run("run-1", RunStatus::Running, None)),
            ("/s/env-days".into(), run("run-2", RunStatus::Paused, None)),
            (
                "/s/env-ageless".into(),
                run("run-3", RunStatus::Running, None),
            ),
        ]
    };
    // Not asked: every one is kept with its run named, as before.
    let rig = Rig::new();
    *rig.host.dirs.lock().unwrap() = dirs();
    *rig.hosted.by_dir.lock().unwrap() = hosted();
    let report = rig
        .use_case()
        .execute(&request(AbandonedRuns::Keep))
        .unwrap();
    for id in ["env-hour", "env-days", "env-ageless"] {
        assert!(
            kept_reason(&report, id).contains("nothing records it: end the run"),
            "{id}: {report:?}"
        );
    }
    // Asked outright: collected, and the reason says the run is abandoned.
    let rig = Rig::new();
    *rig.host.dirs.lock().unwrap() = dirs();
    *rig.hosted.by_dir.lock().unwrap() = hosted();
    let report = rig
        .use_case()
        .execute(&request(AbandonedRuns::Collect))
        .unwrap();
    for id in ["env-hour", "env-days", "env-ageless"] {
        let candidate = report
            .removable
            .iter()
            .find(|c| c.environment_id == id)
            .unwrap_or_else(|| panic!("{id}: {report:?}"));
        assert!(
            matches!(candidate.removal, GcRemoval::ConfiguredCleanup { .. }),
            "{id}: {report:?}"
        );
        assert!(
            candidate.reason.contains("no registry record")
                && candidate.reason.contains("abandoned swarm run")
                && candidate.reason.contains("--abandoned"),
            "{id}: {}",
            candidate.reason
        );
    }
    // Asked by age: only a directory at least that old, and one whose age
    // cannot be read is never old enough.
    let rig = Rig::new();
    *rig.host.dirs.lock().unwrap() = dirs();
    *rig.hosted.by_dir.lock().unwrap() = hosted();
    let report = rig
        .use_case()
        .execute(&request(AbandonedRuns::OlderThan {
            secs: 86_400,
            spelled: "1d".into(),
        }))
        .unwrap();
    let days = report
        .removable
        .iter()
        .find(|c| c.environment_id == "env-days")
        .unwrap_or_else(|| panic!("{report:?}"));
    assert!(
        matches!(days.removal, GcRemoval::ConfiguredCleanup { .. }),
        "{report:?}"
    );
    assert_eq!(
        days.reason,
        "container quecto-env-days gone or exited; no registry record; its checkout hosts abandoned swarm run run-2 (paused), 259200s old, collected on --abandoned-after 1d"
    );
    assert!(
        kept_reason(&report, "env-hour").contains("younger than the 1d --abandoned-after"),
        "{report:?}"
    );
    assert!(
        kept_reason(&report, "env-ageless").contains("age could not be read"),
        "{report:?}"
    );
}

/// The flag reaches only what nothing records: a `stopped` record's
/// directory hosting an unfinished run is the registry's business, and an
/// unreadable store is never an abandoned run.
#[test]
fn a_recorded_directory_and_an_unreadable_store_are_never_abandoned() {
    use crate::domain::environment_retention::SwarmRunObservation;
    let rig = Rig::new();
    rig.registry
        .commit(record("C2", "env-recorded", EnvironmentStatus::Stopped));
    *rig.host.dirs.lock().unwrap() = vec![
        dir("env-recorded", Some("quecto-env-recorded")),
        dir("env-unreadable", Some("quecto-env-unreadable")),
    ];
    *rig.hosted.by_id.lock().unwrap() = vec![(
        "env-recorded".into(),
        run("run-r", RunStatus::Running, None),
    )];
    *rig.hosted.by_dir.lock().unwrap() = vec![(
        "/s/env-unreadable".into(),
        SwarmRunObservation::Unreadable("database is locked".into()),
    )];
    let report = rig
        .use_case()
        .execute(&request(AbandonedRuns::Collect))
        .unwrap();
    assert!(
        kept_reason(&report, "env-recorded").contains("hosts swarm run run-r (running)"),
        "{report:?}"
    );
    assert!(
        kept_reason(&report, "env-unreadable").contains("could not be read"),
        "{report:?}"
    );
}
