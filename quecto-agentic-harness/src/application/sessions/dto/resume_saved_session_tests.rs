use super::*;
use crate::application::sessions::dto::FleetSettlementOutcome;
use crate::domain::ids::AgentUuid;

#[test]
fn a_typed_legacy_name_denotes_the_cli_session_of_that_name() {
    let target = ResumeTarget::parse("  saved-one ").unwrap();
    assert_eq!(target.name, "saved-one");
    assert_eq!(target.identity.runtime_key(), "cli:saved-one");
}

#[test]
fn an_already_qualified_cli_key_is_kept_and_never_re_prefixed() {
    let target = ResumeTarget::parse("cli:legacy_1").unwrap();
    assert_eq!(target.name, "cli:legacy_1");
    assert_eq!(target.identity.runtime_key(), "cli:legacy_1");
}

#[test]
fn a_full_user_chat_key_is_the_picker_selection_itself() {
    let target = ResumeTarget::parse("chat-1750000000-abc").unwrap();
    assert_eq!(target.name, "chat-1750000000-abc");
    assert_eq!(target.identity.runtime_key(), "chat-1750000000-abc");
}

#[test]
fn every_other_spelling_is_refused_with_the_allowlist_text() {
    for rejected in [
        "bad name!",
        "",
        "   ",
        "cli:",
        "cli:bad name",
        "cli:a:b",
        "chat-with space",
        "telegram:123",
        "a/b",
    ] {
        let err = ResumeTarget::parse(rejected).unwrap_err();
        assert!(
            matches!(err, ResumeSavedSessionError::InvalidName),
            "{rejected:?} → {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "session name must contain only alphanumeric, '-', or '_'"
        );
    }
}

#[test]
fn every_error_text_is_the_wire_text_the_transports_present() {
    assert_eq!(
        ResumeSavedSessionError::Ephemeral.to_string(),
        "cannot resume sessions in ephemeral mode"
    );
    assert_eq!(
        ResumeSavedSessionError::Refused(SessionTransitionRefused::Interrupted).to_string(),
        "subagent teardown was interrupted; the current session was kept"
    );
    assert_eq!(
        ResumeSavedSessionError::Save(SaveSessionError::Store(DomainError::Session(
            "disk full".into()
        )))
        .to_string(),
        "failed to save current session: session error: disk full"
    );
    assert_eq!(
        ResumeSavedSessionError::Claim(DomainError::Session("owned by pid 7".into())).to_string(),
        "session error: owned by pid 7"
    );
    assert_eq!(
        ResumeSavedSessionError::NotFound("cli:gone".into()).to_string(),
        "session not found: cli:gone"
    );
    assert_eq!(
        ResumeSavedSessionError::Load(DomainError::Session("corrupt".into())).to_string(),
        "failed to load session: session error: corrupt"
    );
    let unsettled = SessionTransitionRefused::Unsettled(vec![(
        AgentUuid::from("u1".to_string()),
        "no ack".to_string(),
    )]);
    assert!(
        ResumeSavedSessionError::Refused(unsettled)
            .to_string()
            .starts_with("1 subagent(s) could not be settled")
    );
    // The DTO is a plain value: what the fleet reported stays typed.
    let _ = FleetSettlementOutcome::Interrupted;
}
