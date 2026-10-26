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

/// Review #2762 (A): a coordinator the parent launched keeps its swarm run
/// live while it is up, idle or not; once it exited it does not.
#[test]
fn issue_2708_liveness_a_launched_coordinator_is_live_until_it_exits() {
    for (status, live) in [
        (SubagentStatus::Idle, WorkLiveness::Live),
        (SubagentStatus::Error, WorkLiveness::Live),
        (SubagentStatus::Exited, WorkLiveness::Empty),
    ] {
        let registry = with(&[("coordinator", None, status.clone())]);
        registry
            .lock()
            .unwrap()
            .get_mut("coordinator")
            .unwrap()
            .coordinator_wake
            .launched = true;
        assert_eq!(registry_liveness(Some(&registry)), live, "{status:?}");
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
