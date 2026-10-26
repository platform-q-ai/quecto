//! #2708 L01–L05: the registry's read of the work a root owns.
use super::*;
use crate::infrastructure::tools::agents::subagent_registry::{SubagentEntry, new_registry};
use crate::infrastructure::tools::agents::subagent_status::SubagentStatus;
use std::path::PathBuf;

fn with(statuses: &[(&str, Option<&str>, SubagentStatus)]) -> SubagentRegistry {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        for (id, parent, status) in statuses {
            let mut entry = SubagentEntry::new(PathBuf::from("/s"), 0);
            entry.parent_id = parent.map(str::to_owned);
            entry.status = status.clone();
            entries.insert((*id).into(), entry);
        }
    }
    registry
}

#[test]
fn issue_2708_liveness_a_starting_child_is_live() {
    let registry = with(&[("c", None, SubagentStatus::Starting)]);
    assert_eq!(registry_liveness(Some(&registry)), WorkLiveness::Live);
}

#[test]
fn issue_2708_liveness_a_running_descendant_under_an_idle_child_is_live() {
    let registry = with(&[
        ("c", None, SubagentStatus::Idle),
        ("gc", Some("c"), SubagentStatus::Running),
    ]);
    assert_eq!(registry_liveness(Some(&registry)), WorkLiveness::Live);
}

#[test]
fn issue_2708_liveness_retained_idle_and_dead_rows_are_empty() {
    let registry = with(&[
        ("idle", None, SubagentStatus::Idle),
        ("errored", None, SubagentStatus::Error),
        ("exited", None, SubagentStatus::Exited),
    ]);
    assert_eq!(registry_liveness(Some(&registry)), WorkLiveness::Empty);
    assert_eq!(registry_liveness(None), WorkLiveness::Empty);
}

/// Review #2762 round 3 (1): an idle coordinator the parent launched holds
/// work only while its last report was a hold. One that never reported (a
/// coordinator that ended its first turn without a board sends none) holds
/// none, nor does one that exited. (The reported kinds are driven
/// through the monitor in the swarm-wake tests.)
#[test]
fn issue_2708_liveness_a_launched_coordinator_holds_work_only_after_a_hold() {
    use crate::domain::swarm::services::parent_wake::WakeKind;
    for (status, report, live) in [
        (SubagentStatus::Idle, None, WorkLiveness::Empty),
        (
            SubagentStatus::Idle,
            Some(WakeKind::Unknown),
            WorkLiveness::Unknown,
        ),
        (
            SubagentStatus::Idle,
            Some(WakeKind::Hold),
            WorkLiveness::Live,
        ),
        // Round 4 (L5): an errored coordinator reads by its wake (a failed
        // turn not yet reported is unknown: see the swarm-wake tests).
        (
            SubagentStatus::Error,
            Some(WakeKind::Finished),
            WorkLiveness::Empty,
        ),
        (
            SubagentStatus::Exited,
            Some(WakeKind::Hold),
            WorkLiveness::Empty,
        ),
    ] {
        let registry = with(&[("coordinator", None, status.clone())]);
        {
            let mut entries = registry.lock().unwrap();
            let wake = &mut entries.get_mut("coordinator").unwrap().coordinator_wake;
            wake.launched = true;
            if let Some(kind) = report {
                wake.report_for_test(kind);
            }
        }
        let case = format!("{status:?} {report:?}");
        assert_eq!(registry_liveness(Some(&registry)), live, "{case}");
    }
}

#[test]
fn issue_2708_liveness_a_poisoned_registry_is_unknown_never_empty() {
    let registry = with(&[("c", None, SubagentStatus::Idle)]);
    let poisoner = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.lock().unwrap();
        std::panic::resume_unwind(Box::new("poison the registry"));
    })
    .join();
    assert!(registry.is_poisoned());
    assert_eq!(registry_liveness(Some(&registry)), WorkLiveness::Unknown);
}
