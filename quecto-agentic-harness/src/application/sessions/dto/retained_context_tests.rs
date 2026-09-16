use super::{INDEX_QUERY, RecallError, RecallQuery, Retained};
use crate::domain::error::DomainError;
use crate::domain::session_identity::SpillId;

#[test]
fn the_reserved_list_id_selects_the_index() {
    assert_eq!(INDEX_QUERY, "list");
    assert_eq!(RecallQuery::parse("list").unwrap(), RecallQuery::Index);
}

#[test]
fn any_other_non_empty_id_selects_that_entry_verbatim() {
    for id in [
        "turn20:bash:0",
        "turn1:msg:assistant:2",
        "LIST",
        " list",
        "x",
    ] {
        assert_eq!(
            RecallQuery::parse(id).unwrap(),
            RecallQuery::Entry(SpillId::new(id)),
            "{id:?}"
        );
    }
}

#[test]
fn the_empty_id_is_malformed() {
    let err = RecallQuery::parse("").unwrap_err();
    assert!(matches!(err, RecallError::MalformedId));
    assert_eq!(err.to_string(), "recall requires a non-empty id");
}

#[test]
fn a_store_error_displays_as_the_domain_error() {
    let err = RecallError::from(DomainError::Session("boom".into()));
    assert!(matches!(err, RecallError::Store(_)));
    assert_eq!(err.to_string(), "session error: boom");
}

#[test]
fn the_receipt_carries_the_retained_id() {
    let receipt = Retained {
        id: "turn1:bash:0".into(),
    };
    assert_eq!(receipt.clone(), receipt);
    assert_eq!(receipt.id, "turn1:bash:0");
}
