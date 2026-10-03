//! #2435: a model the provider refused for the account in use is recorded
//! once, per store, and every later resolve publishes it as not runnable
//! with the provider's reason.

use super::*;

fn reference(qualified: &str) -> ModelRef {
    ModelRef::parse_qualified(qualified).unwrap()
}

const REASON: &str = "The 'mini' model is not supported when using Codex with a ChatGPT account.";

#[test]
fn a_refusal_is_recorded_once_and_held_by_every_clone_of_the_store() {
    let store = CatalogueSnapshotStore::empty();
    let shared = store.clone();
    assert_eq!(store.refusal(&reference("openai-oauth/mini")), None);
    assert!(store.record_refusal(&reference("openai-oauth/mini"), REASON));
    assert!(
        !shared.record_refusal(&reference("openai-oauth/mini"), "a later reason"),
        "the second refusal of the same model is not new"
    );
    assert_eq!(
        shared.refusal(&reference("openai-oauth/mini")).as_deref(),
        Some(REASON),
        "the first reason is kept"
    );
    assert_eq!(
        store.refusal(&reference("openai-api/mini")),
        None,
        "another provider (auth mode) of the same model is not refused"
    );
    assert_eq!(
        CatalogueSnapshotStore::empty().refusal(&reference("openai-oauth/mini")),
        None,
        "another store knows nothing of it"
    );
}

#[test]
fn every_later_resolve_publishes_a_refused_model_as_not_runnable_with_the_reason() {
    let source = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![
            entry("openai-oauth", "mini", "Mini"),
            entry("openai-oauth", "sol", "Sol"),
            entry("openai-api", "mini", "Mini"),
        ],
    );
    let credentials = FakeCredentials::granting(&["openai-oauth", "openai-api"]);
    let store = CatalogueSnapshotStore::empty();
    store.record_refusal(&reference("openai-oauth/mini"), REASON);
    for _ in 0..2 {
        let resolved =
            ResolveCatalogueUseCase.resolve_and_publish(&[&source], &credentials, &store);
        let availability = |qualified: &str| {
            resolved
                .snapshot
                .find(&reference(qualified))
                .unwrap()
                .model
                .availability
                .clone()
        };
        let refused = availability("openai-oauth/mini");
        assert!(!refused.is_runnable());
        assert_eq!(
            refused.reasons(),
            &[UnavailableReason::RefusedForAccount(REASON.to_string())]
        );
        assert!(availability("openai-oauth/sol").is_runnable());
        assert!(availability("openai-api/mini").is_runnable());
    }
}

#[test]
fn a_refused_model_without_a_credential_names_both_reasons() {
    let source = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![entry("openai-oauth", "mini", "Mini")],
    );
    let store = CatalogueSnapshotStore::empty();
    store.record_refusal(&reference("openai-oauth/mini"), REASON);
    let resolved = ResolveCatalogueUseCase.resolve_and_publish(
        &[&source],
        &FakeCredentials::granting(&[]),
        &store,
    );
    let entry = resolved
        .snapshot
        .find(&reference("openai-oauth/mini"))
        .unwrap();
    assert_eq!(
        entry.model.availability.reasons(),
        &[
            UnavailableReason::MissingCredential,
            UnavailableReason::RefusedForAccount(REASON.to_string()),
        ]
    );
}
