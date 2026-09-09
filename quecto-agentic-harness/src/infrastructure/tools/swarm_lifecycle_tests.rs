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
