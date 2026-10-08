//! Tests for the provider-slot rules the runtime factory builds by, and
//! the routed slots the catalogue credits from (#2451).

use super::*;
use crate::domain::catalogue::value_objects::catalogue::ProviderId;

fn credential(
    vendor: &str,
    method: AuthMethod,
    token: &str,
    expires_at: Option<i64>,
) -> Credential {
    Credential {
        provider: vendor.to_string(),
        token: token.to_string(),
        method,
        expires_at,
        refresh_token: None,
        account_id: None,
    }
}

fn oauth(vendor: &str) -> AuthIdentity {
    AuthIdentity::OAuth {
        provider: Some(ProviderId::new(vendor.to_string()).unwrap()),
    }
}

fn routed(routes: &[&str]) -> ProviderSlots {
    ProviderSlots::routed(routes.iter().map(|route| (*route).to_string()))
}

#[test]
fn an_api_slot_takes_the_configured_key_before_any_stored_token() {
    let stored = credential("openai", AuthMethod::Token, "stored", None);
    assert_eq!(
        api_slot_key("configured", Some(stored.clone())).as_deref(),
        Some("configured")
    );
    assert_eq!(api_slot_key("", Some(stored)).as_deref(), Some("stored"));
    assert_eq!(api_slot_key("", None), None);
}

#[test]
fn an_api_slot_refuses_an_expired_empty_or_oauth_stored_credential() {
    let expired = credential("openai", AuthMethod::Token, "stored", Some(0));
    let empty = credential("openai", AuthMethod::Token, "", None);
    let oauth = credential("openai", AuthMethod::OAuth, "signed-in", None);
    assert_eq!(api_slot_key("", Some(expired)), None);
    assert_eq!(api_slot_key("", Some(empty)), None);
    assert_eq!(
        api_slot_key("", Some(oauth)),
        None,
        "a sign-in is not an API key"
    );
}

#[test]
fn an_oauth_slot_takes_a_stored_oauth_token_even_when_expired() {
    let fresh = credential("anthropic", AuthMethod::OAuth, "fresh", Some(i64::MAX));
    let expired = credential("anthropic", AuthMethod::OAuth, "stale", Some(0));
    assert_eq!(oauth_slot_token(Some(fresh)).as_deref(), Some("fresh"));
    assert_eq!(oauth_slot_token(Some(expired)).as_deref(), Some("stale"));
}

#[test]
fn an_oauth_slot_refuses_a_token_credential_or_an_empty_token() {
    let token = credential("anthropic", AuthMethod::Token, "api", None);
    let empty = credential("anthropic", AuthMethod::OAuth, "", None);
    assert_eq!(
        oauth_slot_token(Some(token)),
        None,
        "an API key is not a sign-in"
    );
    assert_eq!(oauth_slot_token(Some(empty)), None);
    assert_eq!(oauth_slot_token(None), None);
}

#[test]
fn a_routed_slot_credits_its_own_models_only() {
    let slots = routed(&["anthropic-oauth", "openai-api"]);
    assert!(slots.credits("anthropic-oauth", &oauth("anthropic")));
    assert!(slots.credits("openai-api", &AuthIdentity::ApiKey));
    assert!(
        !slots.credits("anthropic-api", &AuthIdentity::ApiKey),
        "an OAuth slot never credits the API slot"
    );
    assert!(!slots.credits("openai-oauth", &oauth("openai")));
}

#[test]
fn routed_oauth_providers_are_credited_whatever_their_name() {
    let slots = routed(&["XAI", " my-claude "]);
    assert!(slots.credits("xai", &oauth("xai")));
    assert!(slots.credits("My-Claude", &oauth("anthropic")));
    assert!(
        !slots.credits("grok", &oauth("xai")),
        "only routed providers"
    );
}

#[test]
fn a_routed_api_key_provider_is_credentialed_by_its_records_not_its_route() {
    let slots = routed(&["keyed"]);
    assert!(
        !slots.credits("keyed", &AuthIdentity::ApiKey),
        "a key on one model never marks its siblings"
    );
}

#[test]
fn no_runtime_credits_nothing() {
    let slots = ProviderSlots::of_runtime(
        &crate::application::provider_runtime::RuntimeSnapshotStore::new(),
    );
    assert_eq!(slots, ProviderSlots::none());
    for slot in DEDICATED_SLOTS {
        assert!(!slots.credits(slot, &AuthIdentity::ApiKey));
    }
    assert!(!slots.credits("xai", &oauth("xai")));
}
