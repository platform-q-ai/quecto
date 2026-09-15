use super::*;
use crate::domain::error::DomainError;

#[test]
fn a_save_failure_presents_the_store_text_and_keeps_the_ledger_position() {
    let ledger = LedgerAdvance {
        epoch: 3,
        rev: 7,
        changed: true,
    };
    let err = ClearConversationError::Save {
        ledger,
        error: SaveSessionError::Store(DomainError::Session("disk full".into())),
    };
    assert_eq!(
        err.to_string(),
        "failed to save cleared session: session error: disk full"
    );
    assert_eq!(err.ledger(), ledger);
    assert!(std::error::Error::source(&err).is_none());
    assert_eq!(
        ClearedConversation { ledger },
        ClearedConversation { ledger }
    );
}
