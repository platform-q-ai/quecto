//! #2247 round 2 L4: what a restore says of one record — retained at
//! restore, could not be corrected or forgotten, what an observing restore
//! would write — goes to the audience that speaks for it: a session for the
//! environments it created (another session's go to the debug log), a
//! fleet-wide command (`quecto container ls|kill|gc`) for every one.
use std::sync::{Arc, Mutex};

use super::super::dto::{EnvironmentLiveness, OvertakenAudience, RestoreMode, RestoredRegistry};
use super::super::ports::EnvironmentProcess;
use super::restore_registry_tests::{
    FakeHosted, FakeStore, failing_store_with, process, process_with_disk, record, store_with,
};
use super::{RestoreRegistry, unfinished_run_reason};
use crate::application::environments::dto::StateOnDisk;
use crate::domain::environment_registry::{EnvironmentStatus, GONE_AT_RESTORE};
use crate::domain::environment_retention::{HostedSwarmRun, SwarmRunObservation};
use crate::domain::swarm::RunStatus;

/// Every record here was created by this session.
const CREATOR: &str = "cli:one";
/// Another session, which created none of them.
const OTHER: &str = "cli:two";

fn unfinished_run() -> HostedSwarmRun {
    HostedSwarmRun {
        id: "run-1".into(),
        status: RunStatus::Running,
        outcome: None,
        coordinator: "coordinator".into(),
        deadline: 4_102_444_800.0,
    }
}

/// C1's checkout hosts an unfinished run.
fn hosting_c1() -> Arc<FakeHosted> {
    Arc::new(FakeHosted {
        by_ref: Mutex::new(vec![(
            "C1".to_string(),
            SwarmRunObservation::Run(unfinished_run()),
        )]),
        asked: Mutex::new(vec![]),
    })
}

fn restore_as(
    store: Arc<FakeStore>,
    process: Arc<dyn EnvironmentProcess>,
    hosted: Arc<FakeHosted>,
    (session, mode, audience): (&str, RestoreMode, OvertakenAudience),
) -> RestoredRegistry {
    let (_, report) = RestoreRegistry::new(store, process, hosted).restore(session, mode, audience);
    report
}

/// Each audience the report must speak to, and whether it does for a
/// record `CREATOR` created.
fn audiences() -> [((&'static str, OvertakenAudience), bool); 3] {
    [
        ((CREATOR, OvertakenAudience::OwnEnvironments), true),
        ((OTHER, OvertakenAudience::OwnEnvironments), false),
        ((OTHER, OvertakenAudience::Fleet), true),
    ]
}

#[test]
fn a_record_retained_at_restore_is_reported_to_its_audience_only() {
    for ((session, audience), spoken) in audiences() {
        let store = store_with(vec![record("C1", EnvironmentStatus::Running)]);
        let (registry, report) = RestoreRegistry::new(
            store.clone(),
            process(|_| EnvironmentLiveness::Gone),
            hosting_c1(),
        )
        .restore(session, RestoreMode::Correct, audience);
        assert_eq!(
            registry.get("C1").map(|seeded| seeded.status),
            Some(EnvironmentStatus::Retained),
            "the correction itself does not depend on who is told"
        );
        let expected: Vec<(String, String)> = match spoken {
            true => vec![("C1".into(), unfinished_run_reason(&unfinished_run()))],
            false => vec![],
        };
        assert_eq!(report.retained, expected, "{session} {audience:?}");
        assert!(report.diagnostics.is_empty(), "{report:?}");
    }
}

#[test]
fn a_correction_that_could_not_be_written_is_reported_to_its_audience_only() {
    for ((session, audience), spoken) in audiences() {
        let report = restore_as(
            failing_store_with(vec![record("C1", EnvironmentStatus::Running)]),
            process(|_| EnvironmentLiveness::Gone),
            Arc::new(FakeHosted {
                by_ref: Mutex::new(vec![]),
                asked: Mutex::new(vec![]),
            }),
            (session, RestoreMode::Correct, audience),
        );
        let expected: Vec<String> = match spoken {
            true => vec!["C1 could not be corrected in the durable registry: disk full".into()],
            false => vec![],
        };
        assert_eq!(report.diagnostics, expected, "{session} {audience:?}");
        assert_eq!(report.stopped, ["C1"], "the verdict is still counted");
    }
}

#[test]
fn a_record_that_could_not_be_forgotten_is_reported_to_its_audience_only() {
    for ((session, audience), spoken) in audiences() {
        let report = restore_as(
            failing_store_with(vec![record("C4", EnvironmentStatus::Stopped)]),
            process_with_disk(|_| EnvironmentLiveness::Gone, |_| StateOnDisk::Absent),
            Arc::new(FakeHosted {
                by_ref: Mutex::new(vec![]),
                asked: Mutex::new(vec![]),
            }),
            (session, RestoreMode::Correct, audience),
        );
        let expected: Vec<String> = match spoken {
            true => vec!["C4 could not be forgotten in the durable registry: disk full".into()],
            false => vec![],
        };
        assert_eq!(report.diagnostics, expected, "{session} {audience:?}");
    }
}

#[test]
fn what_an_observing_restore_would_write_is_reported_to_its_audience_only() {
    for ((session, audience), spoken) in audiences() {
        let report = restore_as(
            store_with(vec![
                record("C1", EnvironmentStatus::Running),
                record("C4", EnvironmentStatus::Stopped),
            ]),
            process_with_disk(
                |_| EnvironmentLiveness::Gone,
                |dir| match dir.ends_with("env-C4") {
                    true => StateOnDisk::Absent,
                    false => StateOnDisk::Present,
                },
            ),
            Arc::new(FakeHosted {
                by_ref: Mutex::new(vec![]),
                asked: Mutex::new(vec![]),
            }),
            (session, RestoreMode::Observe, audience),
        );
        let expected: Vec<String> = match spoken {
            true => vec![
                format!("C1 would be recorded stopped ({GONE_AT_RESTORE}); not written: this restore only observes"),
                "C4 would be forgotten (stopped; nothing left on disk or in the runtime); not written: this restore only observes".into(),
            ],
            false => vec![],
        };
        assert_eq!(report.diagnostics, expected, "{session} {audience:?}");
    }
}
