use super::*;

// --- expires_at_with_margin tests (issue #256) ---

#[test]
fn test_expires_at_with_margin_subtracts_300_seconds() {
    let now = crate::infrastructure::time::unix_timestamp_secs();
    let result = expires_at_with_margin(3600);
    // Should be approximately now + 3600 - 300 = now + 3300
    let expected = now + 3300;
    assert!(
        (result - expected).abs() <= 2,
        "expected ~{}, got {} (diff: {})",
        expected,
        result,
        (result - expected).abs()
    );
}

#[test]
fn test_expires_at_with_margin_short_expiry() {
    let now = crate::infrastructure::time::unix_timestamp_secs();
    let result = expires_at_with_margin(600);
    let expected = now + 300;
    assert!(
        (result - expected).abs() <= 2,
        "expected ~{}, got {}",
        expected,
        result
    );
}

#[test]
fn test_expires_at_with_margin_zero_expiry() {
    let now = crate::infrastructure::time::unix_timestamp_secs();
    let result = expires_at_with_margin(0);
    // Should be now - 300 (already expired with margin)
    let expected = now - 300;
    assert!(
        (result - expected).abs() <= 2,
        "expected ~{}, got {}",
        expected,
        result
    );
}

fn frontmatter(name: &str, desc: &str, body: &str) -> String {
    format!("---\nname: {}\ndescription: {}\n---\n{}", name, desc, body)
}

#[test]
fn test_load_skill_prompt_with_skills() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        frontmatter("weather", "Weather forecasts", "Fetch weather data"),
    )
    .unwrap();
    let prompt = load_skill_prompt(tmp.path());
    assert_eq!(prompt, "Fetch weather data");
}

#[test]
fn test_load_skill_prompt_empty_when_no_skills() {
    let tmp = tempfile::TempDir::new().unwrap();
    let prompt = load_skill_prompt(tmp.path());
    assert!(prompt.is_empty());
}

#[test]
fn test_load_skill_prompt_skips_invalid_frontmatter() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skill_dir = tmp.path().join("workspace").join("skills").join("bad");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "No frontmatter").unwrap();
    let prompt = load_skill_prompt(tmp.path());
    assert!(prompt.is_empty());
}

#[test]
fn test_merge_prompts_skill_only() {
    let result = merge_prompts("Skill content", &None);
    assert_eq!(result, "Skill content");
}

#[test]
fn test_merge_prompts_skill_and_user() {
    let result = merge_prompts("Skill content", &Some("User prompt".to_string()));
    assert_eq!(result, "Skill content\n\nUser prompt");
}

#[test]
fn test_merge_prompts_skill_with_empty_user() {
    let result = merge_prompts("Skill content", &Some(String::new()));
    assert_eq!(result, "Skill content");
}

#[test]
fn test_datetime_preamble_contains_current_date() {
    let preamble = datetime_preamble();
    assert!(
        preamble.starts_with("Current date and time:"),
        "expected preamble to start with 'Current date and time:', got: {}",
        preamble
    );
    // Should contain a year (4 digits)
    // Approximate current year from epoch seconds
    let ts = crate::infrastructure::time::unix_timestamp_secs();
    let year = (1970 + ts / 31_557_600).to_string();
    assert!(
        preamble.contains(&year),
        "expected preamble to contain current year {}, got: {}",
        year,
        preamble
    );
}

#[test]
fn test_build_system_prompt_datetime_only() {
    let result = build_system_prompt("", &None);
    assert!(result.starts_with("Current date and time:"));
    // No trailing skills/user content
    assert!(!result.contains("\n\n"));
}

#[test]
fn test_build_system_prompt_with_skills() {
    let result = build_system_prompt("Skill content", &None);
    assert!(result.starts_with("Current date and time:"));
    assert!(result.contains("Skill content"));
}

#[test]
fn test_build_system_prompt_with_skills_and_user() {
    let result = build_system_prompt("Skill content", &Some("Be helpful".to_string()));
    assert!(result.starts_with("Current date and time:"));
    assert!(result.contains("Skill content"));
    assert!(result.contains("Be helpful"));
}

#[test]
fn test_build_system_prompt_with_user_only() {
    let result = build_system_prompt("", &Some("Be helpful".to_string()));
    assert!(result.starts_with("Current date and time:"));
    assert!(result.contains("Be helpful"));
}

/// Issue #104: The quecto datetime preamble is intentionally richer than
/// provider-injected "Current date:" metadata. It includes day-of-week,
/// full time with seconds, and timezone — critical for cron scheduling
/// and time-aware tasks.
#[test]
fn test_datetime_preamble_includes_day_of_week_time_and_timezone() {
    let preamble = datetime_preamble();

    // Must include a day-of-week name
    let days = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    assert!(
        days.iter().any(|d| preamble.contains(d)),
        "preamble should include day-of-week, got: {}",
        preamble
    );

    // Must include AM/PM time with seconds (e.g. "06:55:58 PM")
    assert!(
        preamble.contains("AM") || preamble.contains("PM"),
        "preamble should include AM/PM time, got: {}",
        preamble
    );

    // Must include colons in the time portion (HH:MM:SS)
    let colon_count = preamble.chars().filter(|c| *c == ':').count();
    assert!(
        colon_count >= 2,
        "preamble should include HH:MM:SS (at least 2 colons), got: {}",
        preamble
    );

    // After AM/PM, there should be a timezone identifier
    let ampm_pos = preamble.find("AM").or_else(|| preamble.find("PM"));
    if let Some(pos) = ampm_pos {
        let after = &preamble[pos + 2..];
        assert!(
            !after.trim().is_empty(),
            "preamble should have timezone after AM/PM, got: {}",
            preamble
        );
    }
}

// --- resolve_api_key_with_refresh_async tests (issue #254, #257) ---

#[tokio::test]
async fn test_resolve_api_key_with_refresh_async_returns_valid_token() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-valid".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt-test".to_string()),
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async("", &store, "anthropic").await;
    assert_eq!(resolved, "sk-ant-oat01-valid");
}

#[tokio::test]
async fn test_resolve_api_key_with_refresh_async_falls_back_to_config_on_no_credential() {
    use crate::infrastructure::auth::credential_store::CredentialStore;
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    let resolved = resolve_api_key_with_refresh_async("sk-config-key", &store, "anthropic").await;
    assert_eq!(resolved, "sk-config-key");
}

#[tokio::test]
async fn test_resolve_api_key_with_refresh_async_refreshes_expired_token() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "sk-ant-oat01-refreshed",
        "refresh_token": "rt-new-refresh",
        "expires_in": 28800
    });

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("rt-old-refresh".to_string()),
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async_with_oauth_config(
        "",
        &store,
        "anthropic",
        &crate::infrastructure::auth::oauth::OAuthConfig::with_base_url(&server.uri()),
    )
    .await;
    assert_eq!(resolved, "sk-ant-oat01-refreshed");

    let creds = store.load_snapshot().unwrap();
    let cred = creds.get("anthropic").unwrap();
    assert_eq!(cred.token, "sk-ant-oat01-refreshed");
    assert_eq!(cred.refresh_token.as_deref(), Some("rt-new-refresh"));
}

#[tokio::test]
async fn test_resolve_api_key_with_refresh_async_falls_back_on_refresh_failure() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid_grant"))
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("rt-bad-refresh".to_string()),
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async_with_oauth_config(
        "sk-ant-config-fallback",
        &store,
        "anthropic",
        &crate::infrastructure::auth::oauth::OAuthConfig::with_base_url(&server.uri()),
    )
    .await;
    assert_eq!(resolved, "sk-ant-config-fallback");
}

#[tokio::test]
async fn test_resolve_api_key_with_refresh_async_preserves_old_refresh_token_when_omitted() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "sk-ant-oat01-new-no-rt",
        "expires_in": 28800
    });

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("rt-original-keep-me".to_string()),
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async_with_oauth_config(
        "",
        &store,
        "anthropic",
        &crate::infrastructure::auth::oauth::OAuthConfig::with_base_url(&server.uri()),
    )
    .await;
    assert_eq!(resolved, "sk-ant-oat01-new-no-rt");

    let creds = store.load_snapshot().unwrap();
    let cred = creds.get("anthropic").unwrap();
    assert_eq!(cred.token, "sk-ant-oat01-new-no-rt");
    assert_eq!(
        cred.refresh_token.as_deref(),
        Some("rt-original-keep-me"),
        "old refresh token should be preserved when server omits it"
    );
}

#[tokio::test]
async fn test_resolve_api_key_with_refresh_async_updates_refresh_token_when_provided() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "sk-ant-oat01-new-with-rt",
        "refresh_token": "rt-brand-new",
        "expires_in": 28800
    });

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("rt-old".to_string()),
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async_with_oauth_config(
        "",
        &store,
        "anthropic",
        &crate::infrastructure::auth::oauth::OAuthConfig::with_base_url(&server.uri()),
    )
    .await;
    assert_eq!(resolved, "sk-ant-oat01-new-with-rt");

    let creds = store.load_snapshot().unwrap();
    let cred = creds.get("anthropic").unwrap();
    assert_eq!(
        cred.refresh_token.as_deref(),
        Some("rt-brand-new"),
        "refresh token should be updated when server provides a new one"
    );
}
