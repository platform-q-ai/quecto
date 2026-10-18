use super::*;

/// #1729: only the outcomes a coordinator may propose end a run as a pause.
#[test]
fn only_proposable_outcomes_end_a_run() {
    for status in [
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::BudgetExhausted,
    ] {
        assert!(status.proposable(), "{status:?}");
        assert!(status.terminal(), "{status:?} closes into a terminal state");
    }
    for status in [
        RunStatus::Setup,
        RunStatus::Running,
        RunStatus::Paused,
        RunStatus::Cancelled,
    ] {
        assert!(!status.proposable(), "{status:?}");
    }
}
