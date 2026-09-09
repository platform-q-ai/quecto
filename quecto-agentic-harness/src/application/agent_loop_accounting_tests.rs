use super::*;

#[test]
fn only_the_ledger_capacity_refusal_is_dropped() {
    assert!(ledger_full(&DomainError::Tool(
        "request diagnostic ledger full; export before starting another run".into()
    )));
    assert!(!ledger_full(&DomainError::Tool(
        "database is locked".into()
    )));
    assert!(!ledger_full(&DomainError::Provider("ledger full".into())));
}
