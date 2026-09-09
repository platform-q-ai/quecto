use super::*;

#[test]
fn every_non_terminal_run_is_supervised_including_paused() {
    assert!(needs_supervision(RunStatus::Setup));
    assert!(needs_supervision(RunStatus::Running));
    assert!(
        needs_supervision(RunStatus::Paused),
        "a member joining a paused run still needs a watcher"
    );
    for terminal in [
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::Cancelled,
    ] {
        assert!(!needs_supervision(terminal), "{terminal:?}");
    }
}

/// #1715: a supervisor tick records participation from the run status, so a
/// member's shared handle follows the run it watches.
#[test]
fn a_supervisor_tick_records_participation_from_the_run() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    let participation = super::super::swarm_bridge::Participation::shared();
    let mut snapshot = context.snapshot().unwrap();
    assert!(!participation.participating());
    super::observe(&context, &mut snapshot, &participation);
    assert!(participation.participating(), "{snapshot:?}");
    assert_eq!(snapshot.status, crate::domain::swarm::RunStatus::Running);
}
