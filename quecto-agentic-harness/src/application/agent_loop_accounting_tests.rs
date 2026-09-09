use super::*;

#[test]
fn only_store_contention_keeps_a_diagnostic_pending() {
    for locked in [
        "database is locked",
        "database table is locked",
        "database schema is locked",
    ] {
        assert_eq!(
            classify_accounting_failure(&DomainError::Tool(locked.into())),
            AccountingFailure::Transient,
            "{locked}"
        );
    }
    for durable in [
        "request diagnostic ledger full; export before starting another run",
        "invoking member is unknown or death confirmed",
    ] {
        assert_eq!(
            classify_accounting_failure(&DomainError::Tool(durable.into())),
            AccountingFailure::Durable,
            "{durable}"
        );
    }
}
