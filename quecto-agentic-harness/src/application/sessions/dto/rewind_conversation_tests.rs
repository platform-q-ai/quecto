use super::*;
use crate::domain::error::DomainError;

#[test]
fn refusals_present_the_wire_texts_verbatim_and_carry_no_ledger() {
    let cases = [
        (
            RewindConversationError::MissingTarget,
            "rewind requires messageId or messageIndex",
        ),
        (
            RewindConversationError::TargetNotFound,
            "rewind target not found",
        ),
        (
            RewindConversationError::AmbiguousLegacyIndex,
            "messageIndex is ambiguous beyond one history page; rewind requires messageId",
        ),
        (
            RewindConversationError::InvalidTarget,
            "invalid rewind target",
        ),
    ];
    for (err, text) in cases {
        assert_eq!(err.to_string(), text);
        assert!(err.ledger().is_none(), "{text}: refused before any change");
        assert!(std::error::Error::source(&err).is_none());
    }
}

#[test]
fn target_errors_map_onto_their_refusals() {
    assert!(matches!(
        RewindConversationError::from(RewindTargetError::NotFound),
        RewindConversationError::TargetNotFound
    ));
    assert!(matches!(
        RewindConversationError::from(RewindTargetError::AmbiguousLegacyIndex),
        RewindConversationError::AmbiguousLegacyIndex
    ));
}

#[test]
fn a_save_failure_presents_the_store_text_and_keeps_the_ledger_position() {
    let ledger = LedgerAdvance {
        epoch: 2,
        rev: 5,
        changed: true,
    };
    let err = RewindConversationError::Save {
        ledger,
        error: SaveSessionError::Store(DomainError::Session("disk full".into())),
    };
    assert_eq!(
        err.to_string(),
        "failed to save rewound session: session error: disk full"
    );
    assert_eq!(err.ledger(), Some(ledger));
    let request = RewindRequest {
        target: RewindTarget::LegacyIndex(1),
        legacy_index_window: 64,
    };
    assert_eq!(request.clone(), request);
    assert_eq!(
        RewoundConversation {
            message_index: 1,
            ledger
        },
        RewoundConversation {
            message_index: 1,
            ledger
        }
    );
}
