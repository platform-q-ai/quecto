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

#[test]
fn test_merge_prompts_user_only() {
    let result = merge_prompts(&Some("User prompt".to_string()));
    assert_eq!(result, "User prompt");
}

#[test]
fn test_merge_prompts_empty_user() {
    let result = merge_prompts(&Some(String::new()));
    assert!(result.is_empty());
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
    let result = build_system_prompt(&None);
    assert!(result.starts_with("Current date and time:"));
    assert!(result.contains("`docs` tool"));
    assert!(result.contains("name `quecto`"));
    assert!(result.contains("extend tools"));
    assert!(result.contains("subagents"));
    assert!(result.contains("workflows"));
    assert!(!result.contains("quecto-tui"));
    assert!(!result.contains("quecto-api"));
    assert!(!result.contains("quecto-mcp"));
}

#[test]
fn test_build_system_prompt_with_user_only() {
    let result = build_system_prompt(&Some("Be helpful".to_string()));
    assert!(result.starts_with("Current date and time:"));
    assert!(result.contains("Be helpful"));
}

#[tokio::test]
async fn workflow_subsystem_registers_live_engine_handle() {
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    let workflow = register_workflow_tool(
        &mut registry,
        crate::domain::workflow::WorkflowConfig::default(),
        false,
        None,
    )
    .unwrap();

    workflow
        .lock()
        .unwrap()
        .set_issue(77, "shared state".into());
    let result = registry
        .execute(
            "workflow",
            r#"{"action":"select_template","template":"feature"}"#,
        )
        .await
        .unwrap();
    assert!(!result.is_error);

    assert_eq!(
        workflow.lock().unwrap().snapshot(true).active_issue,
        Some((77, "shared state".into()))
    );
}

#[test]
fn append_workflow_prompt_if_active_skips_selector_mode_unless_forced() {
    let workflow = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            false,
        )
        .unwrap(),
    ));

    let mut prompt = "base".to_string();
    append_workflow_prompt_if_active(&mut prompt, &workflow, false);
    assert_eq!(prompt, "base");

    append_workflow_prompt_if_active(&mut prompt, &workflow, true);
    assert!(prompt.contains("MODE: SELECT TEMPLATE"));
}

#[test]
fn append_workflow_prompt_if_active_appends_after_template_selection() {
    let workflow = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            false,
        )
        .unwrap(),
    ));
    workflow
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();

    let mut prompt = "base".to_string();
    append_workflow_prompt_if_active(&mut prompt, &workflow, false);
    assert!(prompt.contains("Template: Feature"));
    assert!(prompt.contains("CURRENT STEP"));
}

#[test]
fn workflow_guard_registered_only_when_guards_enabled() {
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    let _handle = register_workflow_tool(
        &mut registry,
        crate::domain::workflow::WorkflowConfig::default(),
        true,
        None,
    )
    .unwrap();
    assert_eq!(
        registry.guard_count(),
        1,
        "guard should be registered when guards_enabled=true"
    );
}

#[test]
fn workflow_guard_not_registered_when_guards_disabled() {
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    let _handle = register_workflow_tool(
        &mut registry,
        crate::domain::workflow::WorkflowConfig::default(),
        false,
        None,
    )
    .unwrap();
    assert_eq!(
        registry.guard_count(),
        0,
        "no guard should be registered when guards_enabled=false"
    );
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

// --- append_extension_prompt tests ---

#[test]
fn test_append_extension_prompt_adds_section() {
    let mut system = "base prompt".to_string();
    append_extension_prompt(&mut system, "ext content");
    assert!(system.contains("## Extensions"));
    assert!(system.contains("ext content"));
    assert!(system.contains("## End Extensions"));
}

#[test]
fn test_append_extension_prompt_empty_is_noop() {
    let mut system = "base prompt".to_string();
    append_extension_prompt(&mut system, "");
    assert_eq!(system, "base prompt");
}

// --- resolve_api_key tests ---

#[test]
fn test_resolve_api_key_uses_cred_when_valid() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential};
    let mut creds = std::collections::HashMap::new();
    creds.insert(
        "anthropic".to_string(),
        Credential {
            provider: "anthropic".to_string(),
            token: "sk-from-cred".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        },
    );
    let key = resolve_api_key("sk-from-config", &creds, "anthropic");
    assert_eq!(key, "sk-from-cred");
}

#[test]
fn test_resolve_api_key_falls_back_when_expired() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential};
    let mut creds = std::collections::HashMap::new();
    creds.insert(
        "anthropic".to_string(),
        Credential {
            provider: "anthropic".to_string(),
            token: "sk-expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0), // expired
            refresh_token: None,
            account_id: None,
        },
    );
    let key = resolve_api_key("sk-from-config", &creds, "anthropic");
    assert_eq!(key, "sk-from-config");
}

#[test]
fn test_resolve_api_key_falls_back_when_no_cred() {
    let creds = std::collections::HashMap::new();
    let key = resolve_api_key("sk-from-config", &creds, "anthropic");
    assert_eq!(key, "sk-from-config");
}

// --- resolve_agent_workspace tests ---

#[test]
fn test_resolve_agent_workspace_sandbox() {
    let result = resolve_agent_workspace("/home/user/workspace", false);
    assert_eq!(result, std::path::PathBuf::from("/home/user/workspace"));
}

#[test]
fn test_resolve_agent_workspace_no_sandbox_uses_cwd() {
    let result = resolve_agent_workspace("/home/user/workspace", true);
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(result, cwd);
}

// --- check_provider_readiness tests ---

#[test]
fn test_check_provider_readiness_empty() {
    let creds = std::collections::HashMap::new();
    let expired = check_provider_readiness(&creds);
    assert!(expired.is_empty());
}

#[test]
fn test_check_provider_readiness_with_expired() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential};
    let mut creds = std::collections::HashMap::new();
    creds.insert(
        "anthropic".to_string(),
        Credential {
            provider: "anthropic".to_string(),
            token: "expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: None,
            account_id: None,
        },
    );
    let expired = check_provider_readiness(&creds);
    assert_eq!(expired, vec!["anthropic"]);
}

#[test]
fn test_check_provider_readiness_valid_not_flagged() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential};
    let mut creds = std::collections::HashMap::new();
    creds.insert(
        "anthropic".to_string(),
        Credential {
            provider: "anthropic".to_string(),
            token: "valid".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        },
    );
    let expired = check_provider_readiness(&creds);
    assert!(expired.is_empty());
}

// --- xdg_runtime_dir_or_temp tests ---

#[test]
fn test_xdg_runtime_dir_or_temp_returns_path() {
    let path = xdg_runtime_dir_or_temp();
    assert!(path.is_dir());
}

// --- build_http_client tests ---

#[test]
fn test_build_http_client_does_not_panic() {
    let _client = build_http_client();
}

// --- OAUTH_EXPIRY_MARGIN_SECS constant ---

#[test]
fn test_oauth_expiry_margin_is_five_minutes() {
    assert_eq!(OAUTH_EXPIRY_MARGIN_SECS, 300);
}
