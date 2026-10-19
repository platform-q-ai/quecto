//! Tests for the provider factory's rebuild and fallback paths.

use super::*;

fn jwt_with_account_id(account_id: &str) -> String {
    use base64::Engine;
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": account_id }
    });
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).unwrap());
    format!("header.{}.signature", enc)
}

#[test]
fn make_provider_factory_builds_anthropic_provider() {
    let factory = make_provider_factory("anthropic", None, reqwest::Client::new());
    let provider = factory("sk-ant-token");
    assert_eq!(provider.name(), "anthropic");
}

#[test]
fn make_provider_factory_openai_plain_token_builds_openai_provider() {
    // A non-JWT token yields no account id, so it falls through to the
    // standard provider constructor rather than the Codex branch.
    let factory = make_provider_factory("openai", None, reqwest::Client::new());
    let provider = factory("plain-non-jwt-token");
    assert_eq!(provider.name(), "openai");
}

#[test]
fn make_provider_factory_invalid_base_falls_back_to_default_base() {
    // A disallowed https host makes the primary build fail; the factory must
    // recover by rebuilding with no custom base instead of panicking.
    let factory = make_provider_factory(
        "anthropic",
        Some("https://evil.not-anthropic.example.com".to_string()),
        reqwest::Client::new(),
    );
    let provider = factory("sk-ant-token");
    assert_eq!(provider.name(), "anthropic");
}

#[test]
fn make_provider_factory_openai_jwt_builds_codex_provider() {
    let factory = make_provider_factory("openai", None, reqwest::Client::new());
    let provider = factory(&jwt_with_account_id("acct-123"));
    // A JWT carrying an account id routes to the Codex provider.
    assert_eq!(provider.name(), "codex");
}

#[test]
fn make_provider_factory_invalid_provider_falls_back_to_openai() {
    let factory = make_provider_factory("not-a-provider", None, reqwest::Client::new());
    let provider = factory("new-token");
    assert_eq!(provider.name(), "openai");
}

#[test]
fn make_provider_factory_retries_codex_with_default_base_for_bad_custom_base() {
    let factory = make_provider_factory(
        "openai",
        Some("http://[::1".to_string()),
        reqwest::Client::new(),
    );
    let provider = factory(&jwt_with_account_id("acct-fallback"));
    assert_eq!(provider.name(), "codex");
}
