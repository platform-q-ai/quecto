//! The collector against the hosted swarm store (round 4 M1b/L2/L3/info,
//! #2033): a `stopped` record's directory, or an unrecorded one, whose
//! checkout hosts a run its owner has not closed is kept whatever the
//! registry says — the board and checkout are the run's; a closed run, the
//! placeholder or no store is collected as before. An own-root directory is judged by
//! the record's own retained inspect; a record an older build relabelled
//! `stopped` while retained is kept; a record seen nowhere is reported.
use super::STATE_GONE_CONTAINER_RUNNING;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::super::dto::{EnvironmentLiveness, EnvironmentStateDir, GcRemoval, GcRequest};
use super::super::ports::HostedSwarmRunInspection;
use super::gc_orphaned_environments::CREATE_GRACE_SECS;
use super::gc_orphaned_environments_tests::{Rig, container, dir, record};
use crate::domain::environment_registry::GONE_AT_RESTORE;
use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentStatus};
use crate::domain::environment_retention::{HostedSwarmRun, SwarmRunObservation};
use crate::domain::swarm::RunStatus;

/// The hosted store as the collector reads it: by environment id for a
/// record, by state dir for an unrecorded directory; what was asked.
#[derive(Default)]
pub(super) struct FakeHosted {
    pub(super) by_id: Mutex<Vec<(String, SwarmRunObservation)>>,
    pub(super) by_dir: Mutex<Vec<(PathBuf, SwarmRunObservation)>>,
    pub(super) asked: Mutex<Vec<String>>,
}

impl HostedSwarmRunInspection for FakeHosted {
    fn inspect_hosted_run(&self, record: &EnvironmentRecord) -> SwarmRunObservation {
        self.asked
            .lock()
            .unwrap()
            .push(format!("record:{}", record.environment_id));
        self.by_id
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| id == &record.environment_id)
            .map(|(_, o)| o.clone())
            .unwrap_or(SwarmRunObservation::NoStore)
    }
    fn inspect_hosted_run_at(&self, state_dir: &Path) -> SwarmRunObservation {
        self.asked
            .lock()
            .unwrap()
            .push(format!("dir:{}", state_dir.display()));
        self.by_dir
            .lock()
            .unwrap()
            .iter()
            .find(|(dir, _)| dir == state_dir)
            .map(|(_, o)| o.clone())
            .unwrap_or(SwarmRunObservation::NoStore)
    }
}

pub(super) fn run(id: &str, status: RunStatus, outcome: Option<RunStatus>) -> SwarmRunObservation {
    SwarmRunObservation::Run(HostedSwarmRun {
        id: id.into(),
        status,
        outcome,
        coordinator: "coordinator".into(),
        deadline: 4_102_444_800.0,
    })
}

pub(super) fn kept_reason<'a>(report: &'a super::super::dto::GcReport, id: &str) -> &'a str {
    report
        .kept
        .iter()
        .find(|k| k.environment_id == id)
        .map(|k| k.reason.as_str())
        .unwrap_or_else(|| panic!("{id} should be kept: {report:?}"))
}

pub(super) fn removable<'a>(report: &'a super::super::dto::GcReport, id: &str) -> &'a GcRemoval {
    report
        .removable
        .iter()
        .find(|c| c.environment_id == id)
        .map(|c| &c.removal)
        .unwrap_or_else(|| panic!("{id} should be removable: {report:?}"))
}

#[test]
fn a_directory_hosting_an_unfinished_run_is_kept_and_one_whose_run_ended_is_collected() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C2", "env-stopped-live", EnvironmentStatus::Stopped));
    rig.registry
        .commit(record("C4", "env-stopped-done", EnvironmentStatus::Stopped));
    *rig.host.dirs.lock().unwrap() = vec![
        dir("env-stopped-live", Some("quecto-env-stopped-live")),
        dir("env-stopped-done", Some("quecto-env-stopped-done")),
        dir("env-orphan-live", Some("quecto-env-orphan-live")),
        dir("env-orphan-paused", None),
        dir("env-orphan-done", Some("quecto-env-orphan-done")),
        dir("env-orphan-placeholder", None),
        dir("env-orphan-nostore", None),
        dir(
            "env-orphan-unreadable",
            Some("quecto-env-orphan-unreadable"),
        ),
    ];
    *rig.hosted.by_id.lock().unwrap() = vec![
        (
            "env-stopped-live".into(),
            run("run-s", RunStatus::Running, None),
        ),
        (
            "env-stopped-done".into(),
            run("run-d", RunStatus::Paused, Some(RunStatus::Failed)),
        ),
    ];
    *rig.hosted.by_dir.lock().unwrap() = vec![
        (
            "/s/env-orphan-live".into(),
            run("run-o", RunStatus::Running, None),
        ),
        (
            "/s/env-orphan-paused".into(),
            run("run-p", RunStatus::Paused, None),
        ),
        (
            "/s/env-orphan-done".into(),
            run("run-e", RunStatus::Succeeded, None),
        ),
        (
            "/s/env-orphan-placeholder".into(),
            SwarmRunObservation::Run(HostedSwarmRun {
                deadline: 0.0,
                ..match run("run-b", RunStatus::Setup, None) {
                    SwarmRunObservation::Run(r) => r,
                    _ => unreachable!(),
                }
            }),
        ),
        (
            "/s/env-orphan-unreadable".into(),
            SwarmRunObservation::Unreadable("database is locked".into()),
        ),
    ];
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    assert_eq!(
        kept_reason(&report, "env-stopped-live"),
        "container quecto-env-stopped-live gone or exited; recorded C2 as stopped, but its checkout hosts swarm run run-s (running); end the run (or remove the directory by hand if its board is unreadable) before it can be collected"
    );
    assert_eq!(
        kept_reason(&report, "env-orphan-live"),
        "container quecto-env-orphan-live gone or exited; no registry record, but its checkout hosts swarm run run-o (running); nothing records it: end the run, or remove the directory by hand, before it can be collected (or pass --abandoned)"
    );
    assert!(
        kept_reason(&report, "env-orphan-paused").contains("hosts swarm run run-p (paused)"),
        "{report:?}"
    );
    assert!(
        kept_reason(&report, "env-orphan-unreadable")
            .contains("hosts a coordination store that could not be read (database is locked); nothing records it"),
        "{report:?}"
    );
    // Paused holding an outcome nobody closed: the finalizer keeps that box
    // (#2070), so the collector does too.
    assert!(
        kept_reason(&report, "env-stopped-done")
            .contains("hosts swarm run run-d (paused holding failed)"),
        "{report:?}"
    );
    for id in [
        "env-orphan-done",
        "env-orphan-placeholder",
        "env-orphan-nostore",
    ] {
        assert!(
            matches!(removable(&report, id), GcRemoval::ConfiguredCleanup { .. }),
            "{id}: {report:?}"
        );
    }
    // A real run removed only those and left the kept directories alone.
    assert_eq!(
        rig.host.removed.lock().unwrap().as_slice(),
        [
            "env-orphan-done",
            "env-orphan-nostore",
            "env-orphan-placeholder"
        ]
    );
    assert!(rig.process.cleaned.lock().unwrap().is_empty());
    let dirs = rig.host.dirs.lock().unwrap();
    for id in [
        "env-stopped-live",
        "env-stopped-done",
        "env-orphan-live",
        "env-orphan-paused",
        "env-orphan-unreadable",
    ] {
        assert!(dirs.iter().any(|d| d.environment_id == id), "{id} kept");
    }
    assert!(report.errors.is_empty(), "{report:?}");
}

#[test]
fn the_store_is_read_only_for_what_would_otherwise_be_collected() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C1", "env-live", EnvironmentStatus::Running));
    rig.registry
        .commit(record("C3", "env-retained", EnvironmentStatus::Retained));
    rig.registry
        .commit(record("C2", "env-stopped", EnvironmentStatus::Stopped));
    *rig.process.liveness.lock().unwrap() = vec![
        ("C1".into(), EnvironmentLiveness::Running),
        ("C3".into(), EnvironmentLiveness::Gone),
    ];
    *rig.host.liveness.lock().unwrap() = vec![
        ("env-live".into(), EnvironmentLiveness::Running),
        ("env-running".into(), EnvironmentLiveness::Running),
    ];
    *rig.host.dirs.lock().unwrap() = vec![
        dir("env-live", Some("quecto-env-live")),
        dir("env-retained", Some("quecto-env-retained")),
        dir("env-stopped", Some("quecto-env-stopped")),
        dir("env-running", Some("quecto-env-running")),
        EnvironmentStateDir {
            age_secs: Some(1),
            ..dir("env-young", None)
        },
        dir("env-orphan", None),
    ];
    *rig.host.containers.lock().unwrap() = vec![container("env-ghost", false)];
    rig.dry_run();
    let mut asked = rig.hosted.asked.lock().unwrap().clone();
    asked.sort();
    assert_eq!(
        asked,
        ["dir:/s/env-orphan", "record:env-stopped"],
        "a running, retained or young one is kept before the store is read; a ghost has no directory"
    );
}

/// Round 4 L2: a directory under a root the record's own cleanup names is
/// judged by that record's own retained inspect, not the config's.
#[test]
fn an_own_root_directory_is_judged_by_the_records_own_inspect() {
    let rig = Rig::new();
    let mut own = record("C7", "env-own", EnvironmentStatus::Stopped);
    own.workspace_path = "/own/env-own/workspace".into();
    own.retained_cleanup_argv = vec!["kill.sh".into(), "--state-dir".into(), "/own".into()];
    rig.registry.commit(own);
    *rig.host.dirs.lock().unwrap() = vec![EnvironmentStateDir {
        path: "/own/env-own".into(),
        environment_id: "env-own".into(),
        container: Some("quecto-env-own".into()),
        age_secs: Some(CREATE_GRACE_SECS * 2),
    }];
    // The config's inspect would call it gone; the record's own says it
    // runs — the record's own is the authority under its own root.
    *rig.process.liveness.lock().unwrap() = vec![("C7".into(), EnvironmentLiveness::Running)];
    let report = rig.dry_run();
    assert_eq!(
        kept_reason(&report, "env-own"),
        "container quecto-env-own is running"
    );
    assert!(report.removable.is_empty(), "{report:?}");
    // And gone by its own inspect: collected through its own cleanup.
    *rig.process.liveness.lock().unwrap() = vec![("C7".into(), EnvironmentLiveness::Gone)];
    let report = rig.dry_run();
    assert_eq!(
        removable(&report, "env-own"),
        &GcRemoval::RetainedCleanup {
            environment_ref: "C7".into()
        }
    );
}

/// Round 4 L3: the signature an older build's restore left on a retained
/// record — `stopped`, its own `retained` reason, the restore's last error
/// — is kept by the collector, whatever the store says now.
#[test]
fn a_record_an_older_build_relabelled_stopped_while_retained_is_kept() {
    let rig = Rig::new();
    let mut stale = record("C9", "env-stale", EnvironmentStatus::Stopped);
    stale.metadata = serde_json::json!({"retained": "run ended: blocked; environment retained for inspection, kill_container to remove"});
    stale.last_error = Some(GONE_AT_RESTORE.into());
    rig.registry.commit(stale);
    // Half the signature: an explicitly killed retained record (its last
    // error cleared) is the collector's.
    let mut killed = record("C8", "env-killed", EnvironmentStatus::Stopped);
    killed.metadata = serde_json::json!({"retained": "run ended: blocked"});
    rig.registry.commit(killed);
    *rig.host.dirs.lock().unwrap() = vec![
        dir("env-stale", Some("quecto-env-stale")),
        dir("env-killed", Some("quecto-env-killed")),
    ];
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    assert_eq!(
        kept_reason(&report, "env-stale"),
        "container quecto-env-stale gone or exited; recorded C9 as stopped, but it was retained; relabelled by an older build — kill explicitly to collect (`quecto container ls` restores it to retained first)"
    );
    assert!(
        !rig.hosted
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.contains("env-stale")),
        "kept before the store is read"
    );
    assert_eq!(rig.process.cleaned.lock().unwrap().as_slice(), ["C8"]);
    assert!(report.errors.is_empty(), "{report:?}");
}

/// Round 4 info: a record of this config seen nowhere — no directory, no
/// container — is reported, never silently skipped: a stopped one is
/// forgotten, any other kept with the reason.
#[test]
fn a_record_seen_nowhere_is_reported_kept_or_forgotten_never_skipped() {
    let rig = Rig::new();
    rig.registry
        .commit(record("C1", "env-gone-running", EnvironmentStatus::Running));
    rig.registry.commit(record(
        "C2",
        "env-gone-retained",
        EnvironmentStatus::Retained,
    ));
    rig.registry
        .commit(record("C3", "env-gone-killing", EnvironmentStatus::Killing));
    rig.registry
        .commit(record("C4", "env-gone-stopped", EnvironmentStatus::Stopped));
    rig.process
        .liveness
        .lock()
        .unwrap()
        .push(("C4".into(), EnvironmentLiveness::Gone));
    // #2134: seen nowhere by this config, but its own inspect finds its
    // container still running (a listing that misses it): kept.
    rig.registry.commit(record(
        "C5",
        "env-unlisted-running",
        EnvironmentStatus::Stopped,
    ));
    rig.process
        .liveness
        .lock()
        .unwrap()
        .push(("C5".into(), EnvironmentLiveness::Running));
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    assert_eq!(
        kept_reason(&report, "env-unlisted-running"),
        format!("recorded C5 as stopped; not forgotten: {STATE_GONE_CONTAINER_RUNNING}")
    );
    assert_eq!(
        kept_reason(&report, "env-gone-running"),
        "recorded C1 as empty; nothing on disk or in the runtime; not collected: kill it (`quecto container kill C1`) so the record is stopped, then gc"
    );
    assert!(
        kept_reason(&report, "env-gone-retained")
            .starts_with("recorded C2 as retained; nothing on disk"),
        "{report:?}"
    );
    assert!(
        kept_reason(&report, "env-gone-killing")
            .starts_with("recorded C3 as killing; nothing on disk"),
        "{report:?}"
    );
    assert_eq!(
        removable(&report, "env-gone-stopped"),
        &GcRemoval::ForgetRecord {
            environment_ref: "C4".into()
        }
    );
    let refs: Vec<String> = rig
        .registry
        .entries()
        .iter()
        .map(|r| r.environment_ref.clone())
        .collect();
    assert_eq!(
        refs,
        ["C1", "C2", "C3", "C5"],
        "only the stopped record that left nothing is forgotten"
    );
    assert!(
        rig.hosted.asked.lock().unwrap().is_empty(),
        "no directory to read a store below"
    );
}
