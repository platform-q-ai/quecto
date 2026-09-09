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

/// Accounting state survives a panic under any of its locks.
#[tokio::test]
async fn poisoned_accounting_locks_are_recovered() {
    use crate::domain::request_observation::{RequestAccounting, RequestObservation};
    let (agent, _) = crate::application::agent_loop::tests::make_agent(vec![], vec![]);
    #[derive(Default)]
    struct Ok_;
    impl RequestAccounting for Ok_ {
        fn record<'a>(
            &'a self,
            _: &'a RequestObservation,
        ) -> crate::domain::subagent_launch::LaunchFuture<'a, Result<(), DomainError>> {
            Box::pin(async { Ok(()) })
        }
    }
    let mut agent = agent.with_request_accounting(Some(std::sync::Arc::new(Ok_)));
    fn poison<T: Send + 'static>(mutex: &std::sync::Mutex<T>) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = mutex.lock().unwrap();
            panic!("poison");
        }));
        assert!(mutex.is_poisoned());
    }
    poison(&agent.unreported_usage);
    poison(&agent.request_observations);
    poison(&agent.accounting_outbox);
    let _ = agent.take_unreported_usage();
    let _ = agent.take_request_diagnostics();
    agent.flush_request_accounting().await.unwrap();
}
