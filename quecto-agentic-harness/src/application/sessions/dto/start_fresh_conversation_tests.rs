use super::*;
use crate::domain::error::DomainError;

#[test]
fn refusals_present_the_exact_transition_texts() {
    let unsettled = SessionTransitionRefused::Unsettled(vec![
        (AgentUuid::new("a"), "ignores TERM".into()),
        (AgentUuid::new("b"), "no exit observed".into()),
    ]);
    assert_eq!(
        unsettled.to_string(),
        "2 subagent(s) could not be settled; the current session was kept: a: ignores TERM; b: no exit observed"
    );
    assert_eq!(
        SessionTransitionRefused::Interrupted.to_string(),
        "subagent teardown was interrupted; the current session was kept"
    );
    assert_eq!(
        SessionTransitionRefused::NoFleetTeardown(3).to_string(),
        "3 live subagent(s) but no fleet teardown is available; the current session was kept"
    );
    assert_eq!(
        SessionTransitionRefused::LiveRowsRemain(1).to_string(),
        "1 live subagent(s) remain after the teardown; the current session was kept"
    );
    assert!(std::error::Error::source(&unsettled).is_none());
}

#[test]
fn start_errors_present_the_refusal_or_the_save_failure_text() {
    let refused = StartFreshConversationError::Refused(SessionTransitionRefused::Interrupted);
    assert_eq!(
        refused.to_string(),
        "subagent teardown was interrupted; the current session was kept"
    );
    let save = StartFreshConversationError::Save(SaveSessionError::Store(DomainError::Session(
        "disk full".into(),
    )));
    assert_eq!(
        save.to_string(),
        "failed to save current session: session error: disk full"
    );
    assert!(std::error::Error::source(&save).is_none());
}

#[test]
fn transition_labels_and_outcomes_are_plain_values() {
    assert_eq!(SessionTransition::Fresh.as_str(), "new_session");
    assert_eq!(SessionTransition::Resume.as_str(), "resume_session");
    let settled = FleetSettled {
        settled: 1,
        pruned: 2,
        joined: false,
        removed: 3,
    };
    assert_eq!(
        FleetSettlementOutcome::Settled(settled),
        FleetSettlementOutcome::Settled(settled)
    );
    assert_ne!(
        FleetSettlementOutcome::Interrupted,
        FleetSettlementOutcome::Unsettled(Vec::new())
    );
    let started = FreshConversationStarted {
        identity: SessionIdentity::fresh_chat(1, 2),
        ledger: LedgerAdvance {
            epoch: 2,
            rev: 0,
            changed: true,
        },
    };
    assert_eq!(started.clone(), started);
    assert_eq!(FleetSettled::default().removed, 0);
}
