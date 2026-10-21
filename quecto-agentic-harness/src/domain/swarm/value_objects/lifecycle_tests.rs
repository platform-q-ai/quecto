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

/// #2685: each exit names the blocker its confirmed death writes, and only
/// that exact text is read back as a death's.
#[test]
fn a_death_blocker_is_read_back_only_by_its_exact_text() {
    for exit in [MemberExit::Orderly, MemberExit::Abrupt] {
        assert_eq!(MemberExit::of_blocker(exit.blocker()), Some(exit));
    }
    for other in [
        "",
        "waiting for the API key",
        "worker death confirmed",
        "Worker death confirmed; coordinator recovery required",
        "worker death confirmed; coordinator recovery required; and more",
        " worker death confirmed; coordinator recovery required",
    ] {
        assert_eq!(MemberExit::of_blocker(other), None, "{other:?}");
    }
}
