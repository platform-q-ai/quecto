use super::*;

#[test]
fn only_the_routine_trigger_allows_a_delta() {
    assert!(!SaveTrigger::Routine.forces_full_save());
    assert!(
        SaveTrigger::Explicit {
            restore_reason: SubagentRestoreReason::LegacyUnspecified
        }
        .forces_full_save()
    );
    assert!(SaveTrigger::OrdinaryExit.forces_full_save());
}

#[test]
fn the_error_presents_the_store_text_verbatim() {
    let err = SaveSessionError::Store(DomainError::Session("disk full".into()));
    assert_eq!(
        err.to_string(),
        DomainError::Session("disk full".into()).to_string()
    );
    assert!(std::error::Error::source(&err).is_none());
    assert_eq!(
        SaveOutcome::Saved {
            mode: SaveMode::Full,
            persisted: 2
        },
        SaveOutcome::Saved {
            mode: SaveMode::Full,
            persisted: 2
        }
    );
}
