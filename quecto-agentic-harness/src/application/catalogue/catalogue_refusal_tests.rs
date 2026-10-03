//! #2435: a model the provider refused for the account in use is recorded
//! once, per store, and every later resolve publishes it as not runnable
//! with the provider's reason.

use super::*;

fn reference(qualified: &str) -> ModelRef {
    ModelRef::parse_qualified(qualified).unwrap()
}

const HELD: std::time::Duration = std::time::Duration::from_secs(3600);

const REASON: &str = "The 'mini' model is not supported when using Codex with a ChatGPT account.";

#[test]
fn a_refusal_is_recorded_once_and_held_by_every_clone_of_the_store() {
    let store = CatalogueSnapshotStore::empty();
    let shared = store.clone();
    assert_eq!(store.refusal(&reference("openai-oauth/mini")), None);
    assert!(store.record_refusal(&reference("openai-oauth/mini"), REASON, HELD));
    assert!(
        !shared.record_refusal(&reference("openai-oauth/mini"), "a later reason", HELD),
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
    store.record_refusal(&reference("openai-oauth/mini"), REASON, HELD);
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
    store.record_refusal(&reference("openai-oauth/mini"), REASON, HELD);
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

/// Review round 1 M3: a refusal expires at the end of its hold, and one
/// recorded after that is new again.
#[test]
fn a_refusal_expires_at_the_end_of_its_hold() {
    let store = CatalogueSnapshotStore::empty();
    let mini = reference("openai-oauth/mini");
    let now = std::time::Instant::now();
    assert!(store.record_refusal(&mini, REASON, HELD));
    assert_eq!(
        store.refusal_at(&mini, now + HELD / 2).as_deref(),
        Some(REASON)
    );
    assert_eq!(
        store.refusal_at(&mini, now + HELD * 2),
        None,
        "the hold has ended"
    );
    let expired = CatalogueSnapshotStore::empty();
    assert!(expired.record_refusal(&mini, REASON, std::time::Duration::ZERO));
    assert_eq!(expired.refusal(&mini), None, "a zero hold is already over");
    assert!(
        expired.record_refusal(&mini, "refused again", HELD),
        "a refusal recorded after the hold ended is new"
    );
    assert_eq!(expired.refusal(&mini).as_deref(), Some("refused again"));
}

/// Review round 1 M3: the provider serving the model releases its refusal.
#[test]
fn a_served_model_is_released() {
    let store = CatalogueSnapshotStore::empty();
    let mini = reference("openai-oauth/mini");
    assert!(!store.clear_refusal(&mini), "nothing held");
    store.record_refusal(&mini, REASON, HELD);
    assert!(store.clear_refusal(&mini));
    assert_eq!(store.refusal(&mini), None);
    assert!(store.record_refusal(&mini, REASON, HELD), "new again");
}
