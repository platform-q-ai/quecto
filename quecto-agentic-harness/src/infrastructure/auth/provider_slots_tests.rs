//! Tests for the provider-slot rules shared by the runtime factory and the
//! catalogue credential status (#2451).

use super::*;
use crate::domain::catalogue::ProviderId;

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

fn probed(keys: ConfiguredApiKeys<'_>, credentials: &[Credential]) -> ProviderSlots {
    let stored = credentials
        .iter()
        .map(|credential| (credential.provider.clone(), credential.clone()))
        .collect();
    ProviderSlots::from_stored(keys, &stored)
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
fn probed_oauth_sign_ins_credit_their_oauth_slots_only() {
    let slots = probed(
        ConfiguredApiKeys::NONE,
        &[
            credential("anthropic", AuthMethod::OAuth, "a", None),
            credential("openai", AuthMethod::OAuth, "o", None),
        ],
    );
    assert!(slots.credits("anthropic-oauth", &oauth("anthropic")));
    assert!(slots.credits("openai-oauth", &oauth("openai")));
    assert!(!slots.credits("anthropic-api", &AuthIdentity::ApiKey));
    assert!(!slots.credits("openai-api", &AuthIdentity::ApiKey));
    assert!(!slots.credits("xai", &oauth("xai")));
}

#[test]
fn probed_api_keys_credit_their_api_slots_only() {
    let slots = probed(
        ConfiguredApiKeys {
            openai: "sk-configured",
            anthropic: "",
        },
        &[credential("anthropic", AuthMethod::Token, "stored", None)],
    );
    assert!(slots.credits("openai-api", &AuthIdentity::ApiKey));
    assert!(slots.credits("anthropic-api", &AuthIdentity::ApiKey));
    assert!(!slots.credits("openai-oauth", &oauth("openai")));
    assert!(!slots.credits("anthropic-oauth", &oauth("anthropic")));
}

#[test]
fn probed_nothing_credits_nothing() {
    let slots = probed(ConfiguredApiKeys::NONE, &[]);
    for (slot, vendor, _) in DEDICATED_SLOTS {
        assert!(!slots.credits(slot, &AuthIdentity::ApiKey));
        assert!(!slots.credits(slot, &oauth(vendor)));
    }
}

#[test]
fn a_probed_sign_in_credits_any_oauth_provider_of_its_vendor_but_no_api_key_provider() {
    let slots = probed(
        ConfiguredApiKeys::NONE,
        &[
            credential("anthropic", AuthMethod::OAuth, "a", None),
            credential("xai", AuthMethod::OAuth, "x", None),
        ],
    );
    assert!(slots.credits("xai", &oauth("xai")));
    assert!(slots.credits("my-claude", &oauth("anthropic")));
    assert!(!slots.credits("my-claude", &AuthIdentity::ApiKey));
    assert!(!slots.credits("my-claude", &AuthIdentity::OAuth { provider: None }));
}

#[test]
fn a_probed_sign_in_for_a_vendor_without_kernel_oauth_credits_nothing() {
    let slots = probed(
        ConfiguredApiKeys::NONE,
        &[credential("acme", AuthMethod::OAuth, "a", None)],
    );
    assert!(!slots.credits("acme-oauth", &oauth("acme")));
}

#[test]
fn routed_slots_credit_the_dedicated_and_oauth_providers_the_runtime_holds() {
    let slots = ProviderSlots::routed([
        "anthropic-oauth".to_string(),
        "XAI".to_string(),
        "keyed".to_string(),
    ]);
    assert!(slots.credits("anthropic-oauth", &oauth("anthropic")));
    assert!(slots.credits("xai", &oauth("xai")));
    assert!(!slots.credits("anthropic-api", &AuthIdentity::ApiKey));
    assert!(
        !slots.credits("keyed", &AuthIdentity::ApiKey),
        "an API-key provider is credentialed by its own records, never its route"
    );
}

#[test]
fn an_unreadable_store_probes_no_slot() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("credentials.json"), "not json").unwrap();
    let slots = ProviderSlots::probe(ConfiguredApiKeys::NONE, &CredentialStore::new(tmp.path()));
    assert_eq!(
        slots,
        ProviderSlots::Probed {
            dedicated: BTreeSet::new(),
            oauth_vendors: BTreeSet::new(),
        }
    );
    let configured = ProviderSlots::probe(
        ConfiguredApiKeys {
            openai: "sk",
            anthropic: "",
        },
        &CredentialStore::new(tmp.path()),
    );
    assert!(
        configured.credits("openai-api", &AuthIdentity::ApiKey),
        "a configured key needs no store"
    );
}
