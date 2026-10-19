use super::*;

#[test]
fn derive_availability_composes_adapter_and_credential_status() {
    let runnable = derive_availability(TransportKind::OpenAiCompletions, true, true);
    assert!(runnable.is_runnable());

    let no_credential = derive_availability(TransportKind::OpenAiCompletions, true, false);
    assert_eq!(no_credential.status(), AvailabilityStatus::Available);
    assert_eq!(
        no_credential.reasons(),
        &[UnavailableReason::MissingCredential]
    );

    let no_adapter = derive_availability(TransportKind::OpenAiCompletions, false, true);
    assert!(!no_adapter.is_runnable());
    assert!(
        no_adapter
            .reasons()
            .iter()
            .any(|reason| matches!(reason, UnavailableReason::UnsupportedTransport { .. }))
    );
}
