use crate::application::sessions::dto::{
    ResumeDecision, ResumeSavedSessionError, ResumeTarget, SaveSessionError,
    SessionTransitionRefused, StartupRefusal, resume_saved_session::ResumeDisposition,
};
use crate::domain::error::DomainError;
use crate::domain::resume_decision::{HomeVersion, ResumeDecisionKind};
use crate::domain::session_home::SessionHomeScope;

fn every_refusal() -> Vec<ResumeSavedSessionError> {
    let decision = ResumeDecision {
        target: ResumeTarget::parse("old").unwrap(),
        kind: ResumeDecisionKind::LegacyUnscoped,
        home_version: HomeVersion::of(
            &ResumeTarget::parse("old").unwrap().identity,
            &SessionHomeScope::LegacyUnscoped,
        ),
        execution_dir: None,
        detail: None,
    };
    vec![
        ResumeSavedSessionError::Busy,
        ResumeSavedSessionError::Ephemeral,
        ResumeSavedSessionError::InvalidName,
        ResumeSavedSessionError::Decision(Box::new(decision)),
        ResumeSavedSessionError::StaleHomeVersion,
        ResumeSavedSessionError::LegacyAction,
        ResumeSavedSessionError::CurrentScopeUnavailable("cwd deleted".into()),
        ResumeSavedSessionError::StartupScope(StartupRefusal {
            key: "cli:old".into(),
            disposition: ResumeDisposition::LegacyUnscoped,
            execution_dir: None,
        }),
        ResumeSavedSessionError::Refused(SessionTransitionRefused::LiveRowsRemain(1)),
        ResumeSavedSessionError::Save(SaveSessionError::Store(DomainError::Session("disk".into()))),
        ResumeSavedSessionError::Claim(DomainError::Session("owned".into())),
        ResumeSavedSessionError::NotFound("nope".into()),
        ResumeSavedSessionError::Load(DomainError::Session("corrupt".into())),
    ]
}

#[test]
fn every_refusal_has_a_distinct_stable_code() {
    let codes: Vec<_> = every_refusal()
        .iter()
        .map(ResumeSavedSessionError::code)
        .collect();
    assert_eq!(
        codes,
        [
            "busy",
            "ephemeral",
            "invalid_name",
            "no_home_recorded",
            "stale_home_version",
            "legacy_action_unsupported",
            "current_scope_unavailable",
            "startup_scope",
            "transition_refused",
            "save_failed",
            "claim_refused",
            "not_found",
            "load_failed",
        ]
    );
}

#[test]
fn every_refusal_reads_as_what_happened_and_what_to_do() {
    let texts: Vec<_> = every_refusal().iter().map(ToString::to_string).collect();
    let expected = [
        "while agent is running",
        "ephemeral mode",
        "alphanumeric",
        "session resume unavailable:",
        "session list out of date; list and choose again",
        "resume actions are no longer supported",
        "current execution directory cannot be discovered",
        "session 'cli:old' cannot start here",
        "live",
        "failed to save current session",
        "owned",
        "session not found: nope",
        "failed to load session",
    ];
    assert_eq!(texts.len(), expected.len());
    for (text, needle) in texts.iter().zip(expected) {
        assert!(text.contains(needle), "{needle:?} in {text:?}");
    }
    // The reason of an undiscoverable cwd is untrusted detail: not echoed.
    assert!(!texts[6].contains("cwd deleted"));
    assert!(texts[6].contains("current execution directory"));
}
