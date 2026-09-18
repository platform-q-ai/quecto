//! The collector's scope (round 3 M1, #2033): a record's workspace never
//! widens the scan on its own. A root is judged only when it is the
//! config's own `--state-dir` (canonically) or the record's own retained
//! cleanup names it — and then only that record's directory, removed
//! through that record's cleanup alone. Anything else is reported as
//! outside the config's state dir: nothing scanned, nothing run.
use std::path::PathBuf;

use super::super::dto::{EnvironmentStateDir, GcRemoval, GcRequest};
use super::gc_orphaned_environments::{CREATE_GRACE_SECS, retained_state_root};
use super::gc_orphaned_environments_tests::{FakeInventory, Rig, container, record};
use crate::domain::environment_registry::EnvironmentStatus;

fn foreign_dir(root: &str, id: &str, container: Option<&str>) -> EnvironmentStateDir {
    EnvironmentStateDir {
        path: PathBuf::from(root).join(id),
        environment_id: id.into(),
        container: container.map(str::to_string),
        age_secs: Some(CREATE_GRACE_SECS * 2),
    }
}

#[test]
fn the_retained_state_root_is_the_state_dir_the_records_own_cleanup_names() {
    let mut record = record("C1", "env-a", EnvironmentStatus::Stopped);
    assert_eq!(
        retained_state_root(&record),
        None,
        "{:?}",
        record.retained_cleanup_argv
    );
    record.retained_cleanup_argv = vec![
        "kill.sh".into(),
        "--state-dir".into(),
        "/own".into(),
        "--op".into(),
        "cleanup".into(),
    ];
    assert_eq!(retained_state_root(&record), Some(PathBuf::from("/own")));
    record.retained_cleanup_argv = vec![
        "kill.sh".into(),
        "--".into(),
        "--state-dir".into(),
        "/x".into(),
    ];
    assert_eq!(
        retained_state_root(&record),
        None,
        "after `--` nothing is an option"
    );
}

/// A stopped record of this config whose workspace lies under a root the
/// config does not own and its own cleanup does not name: reported,
/// its root never scanned, no cleanup (retained or configured) ever run
/// for it — even when the runtime lists its container as exited.
#[test]
fn a_record_outside_the_configs_state_dir_is_reported_and_never_scanned_or_removed() {
    let rig = Rig::new();
    let mut foreign = record("C5", "env-foreign", EnvironmentStatus::Stopped);
    foreign.workspace_path = "/elsewhere/env-foreign/workspace".into();
    rig.registry.commit(foreign);
    let mut nowhere = record("C6", "env-nowhere", EnvironmentStatus::Stopped);
    nowhere.workspace_path = "/scratch/checkout".into();
    rig.registry.commit(nowhere);
    *rig.host.dirs.lock().unwrap() = vec![foreign_dir(
        "/elsewhere",
        "env-foreign",
        Some("quecto-env-foreign"),
    )];
    *rig.host.containers.lock().unwrap() = vec![container("env-foreign", false)];
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    assert_eq!(report.state_roots, vec![PathBuf::from("/s")], "{report:?}");
    assert_eq!(
        rig.host.scanned.lock().unwrap().as_slice(),
        [PathBuf::from("/s")],
        "the foreign root is never listed"
    );
    assert!(report.removable.is_empty(), "{report:?}");
    assert!(report.removed.is_empty(), "{report:?}");
    let kept: Vec<(&str, &str)> = report
        .kept
        .iter()
        .map(|k| (k.environment_id.as_str(), k.reason.as_str()))
        .collect();
    assert_eq!(kept.len(), 2, "{kept:?}");
    assert_eq!(kept[0].0, "env-foreign");
    assert!(
        kept[0].1.contains("recorded C5 at /elsewhere")
            && kept[0]
                .1
                .contains("outside this config's state dir /s; not collected"),
        "{kept:?}"
    );
    assert_eq!(kept[1].0, "env-nowhere");
    assert!(
        kept[1].1.contains("recorded C6 at /scratch/checkout")
            && kept[1]
                .1
                .contains("outside this config's state dir /s; not collected"),
        "{kept:?}"
    );
    assert!(
        rig.process.cleaned.lock().unwrap().is_empty(),
        "no retained cleanup ran"
    );
    assert!(
        rig.host.removed.lock().unwrap().is_empty(),
        "the config's cleanup never ran"
    );
    assert_eq!(rig.registry.entries().len(), 2, "nothing forgotten");
    assert!(report.errors.is_empty(), "{report:?}");
}

/// A root the record's own retained cleanup names is scanned for that
/// record's directory alone and removed through that cleanup alone; the
/// other directories under it are not this config's to judge.
#[test]
fn a_root_the_records_own_cleanup_names_is_judged_for_that_record_alone() {
    let rig = Rig::new();
    let mut own = record("C7", "env-own", EnvironmentStatus::Stopped);
    own.workspace_path = "/own/env-own/workspace".into();
    own.retained_cleanup_argv = vec!["kill.sh".into(), "--state-dir".into(), "/own".into()];
    rig.registry.commit(own);
    // Same root, but its cleanup names another: not admitted.
    let mut liar = record("C8", "env-liar", EnvironmentStatus::Stopped);
    liar.workspace_path = "/own/env-liar/workspace".into();
    liar.retained_cleanup_argv = vec!["kill.sh".into(), "--state-dir".into(), "/other".into()];
    rig.registry.commit(liar);
    *rig.host.dirs.lock().unwrap() = vec![
        foreign_dir("/own", "env-own", Some("quecto-env-own")),
        foreign_dir("/own", "env-liar", Some("quecto-env-liar")),
        foreign_dir("/own", "env-stranger", Some("quecto-env-stranger")),
    ];
    let report = rig.use_case().execute(&GcRequest::default()).unwrap();
    assert_eq!(
        report.state_roots,
        vec![PathBuf::from("/own"), PathBuf::from("/s")],
        "{report:?}"
    );
    let removed: Vec<(&str, &GcRemoval)> = report
        .removed
        .iter()
        .map(|c| (c.environment_id.as_str(), &c.removal))
        .collect();
    assert_eq!(
        removed,
        [(
            "env-own",
            &GcRemoval::RetainedCleanup {
                environment_ref: "C7".into()
            }
        )],
        "{report:?}"
    );
    assert_eq!(rig.process.cleaned.lock().unwrap().as_slice(), ["C7"]);
    assert!(
        rig.host.removed.lock().unwrap().is_empty(),
        "the config's cleanup never runs under a record's own root"
    );
    let kept: Vec<&str> = report
        .kept
        .iter()
        .map(|k| k.environment_id.as_str())
        .collect();
    assert_eq!(
        kept,
        ["env-liar"],
        "the stranger is not judged at all: {report:?}"
    );
    assert!(
        report.kept[0]
            .reason
            .contains("outside this config's state dir"),
        "{report:?}"
    );
    assert!(report.errors.is_empty(), "{report:?}");
}

/// The config's root is compared canonically: a record whose workspace
/// names the root through a symlink is under it, and the root is scanned
/// once.
#[test]
fn a_records_root_that_resolves_to_the_configs_is_the_configs() {
    let rig = Rig::over(FakeInventory {
        aliases: vec![(PathBuf::from("/link"), PathBuf::from("/s"))],
        ..Default::default()
    });
    let mut linked = record("C9", "env-linked", EnvironmentStatus::Stopped);
    linked.workspace_path = "/link/env-linked/workspace".into();
    rig.registry.commit(linked);
    *rig.host.dirs.lock().unwrap() =
        vec![foreign_dir("/s", "env-linked", Some("quecto-env-linked"))];
    let report = rig.dry_run();
    assert_eq!(report.state_roots, vec![PathBuf::from("/s")], "{report:?}");
    assert_eq!(
        rig.host.scanned.lock().unwrap().as_slice(),
        [PathBuf::from("/s")]
    );
    let removable: Vec<(&str, &GcRemoval)> = report
        .removable
        .iter()
        .map(|c| (c.environment_id.as_str(), &c.removal))
        .collect();
    assert_eq!(
        removable,
        [(
            "env-linked",
            &GcRemoval::RetainedCleanup {
                environment_ref: "C9".into()
            }
        )],
        "{report:?}"
    );
    assert!(report.kept.is_empty(), "{report:?}");
}
