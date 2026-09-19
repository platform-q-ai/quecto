use crate::application::sessions::dto::{
    ResumeActionCapabilities, ResumeActionOffer, ResumeDecision, ResumeSavedSessionError,
    ResumeTarget, SaveSessionError, SessionTransitionRefused, StartupRefusal,
    resume_saved_session::ResumeDisposition,
};
use crate::domain::error::DomainError;
use crate::domain::resume_decision::{HomeVersion, ResumeAction, ResumeDecisionKind};
use crate::domain::session_home::SessionHomeScope;

fn every_refusal() -> Vec<ResumeSavedSessionError> {
    let kind = ResumeDecisionKind::LegacyUnscoped;
    let capabilities = ResumeActionCapabilities::cancel_only();
    let decision = ResumeDecision {
        target: ResumeTarget::parse("old").unwrap(),
        kind,
        home_version: HomeVersion::of(
            &ResumeTarget::parse("old").unwrap().identity,
            &SessionHomeScope::LegacyUnscoped,
        ),
        execution_dir: None,
        detail: None,
        offers: kind
            .offered_actions()
            .iter()
            .map(|action| ResumeActionOffer {
                action: *action,
                availability: capabilities.availability(*action),
            })
            .collect(),
    };
    vec![
        ResumeSavedSessionError::Busy,
        ResumeSavedSessionError::HomeVersionRequired(ResumeAction::Locate),
        ResumeSavedSessionError::Ephemeral,
        ResumeSavedSessionError::InvalidName,
        ResumeSavedSessionError::Decision(Box::new(decision)),
        ResumeSavedSessionError::StaleHomeVersion,
        ResumeSavedSessionError::CurrentScopeUnavailable("cwd deleted".into()),
        ResumeSavedSessionError::ActionUnavailable {
            action: ResumeAction::ForkCurrent,
            reason: "not yet".into(),
        },
        ResumeSavedSessionError::ActionExecutedElsewhere(ResumeAction::OpenOriginal),
        ResumeSavedSessionError::ActionNotOffered(ResumeAction::Associate),
        ResumeSavedSessionError::StartupScope(StartupRefusal {
            key: "cli:old".into(),
            disposition: ResumeDisposition::LegacyUnscoped,
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
            "home_version_required",
            "ephemeral",
            "invalid_name",
            "decision_required",
            "stale_home_version",
            "current_scope_unavailable",
            "action_unavailable",
            "action_executed_elsewhere",
            "action_not_offered",
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
        "locate needs expectedHomeVersion",
        "ephemeral mode",
        "alphanumeric",
        "session resume unavailable: legacy session requires explicit first association",
        "session list out of date; list and choose again",
        "current execution directory cannot be discovered",
        "fork_current is unavailable: not yet",
        "open_original is not a restore",
        "associate is not offered for this session",
        "session 'cli:old' cannot start here",
        "live",
        "failed to save current session",
        "owned",
        "session not found: nope",
        "failed to load session",
    ];
    for (text, needle) in texts.iter().zip(expected) {
        assert!(text.contains(needle), "{needle:?} in {text:?}");
    }
    // The reason of an undiscoverable cwd is untrusted detail: not echoed.
    assert!(!texts[6].contains("cwd deleted"));
}
