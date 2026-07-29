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
fn test_build_system_prompt_docs_policy_only() {
    let result = build_system_prompt(&None);
    assert!(!result.contains("Current date and time:"));
    assert!(result.starts_with(agent_role_preamble()));
    assert!(result.contains("Parent Agent"));
    assert!(result.contains("`docs` tool"));
    assert!(result.contains("operating manual"));
    assert!(result.contains("quick-start"));
    assert!(result.contains("definitive source"));
    assert!(!result.contains("name `quecto`"));
    assert!(!result.contains("quecto-tui"));
    assert!(!result.contains("quecto-api"));
    assert!(!result.contains("quecto-mcp"));
}

#[test]
fn test_build_system_prompt_with_user_only() {
    let result = build_system_prompt(&Some("Be helpful".to_string()));
    assert!(!result.contains("Current date and time:"));
    assert!(result.starts_with(agent_role_preamble()));
    assert!(result.contains(agent_docs_retrieval_policy()));
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

// --- #1044/#1045/#1046: config context knobs thread into the loop ---
// The knobs are AgentLoopConfig constructor fields (PR #1048 follow-up), so
// these tests build the loop the way production sites do: config values at
// construction, exercised through the real process() path.

mod context_settings {
    use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use crate::domain::agent::AgentLoop;
    use crate::domain::message::Message;
    use crate::infrastructure::config::AgentDefaults;
    use crate::infrastructure::tools::registry::ToolRegistryImpl;

    /// Build a loop the way production sites do: the context knobs come from
    /// `AgentDefaults` as constructor fields.
    fn agent_with(
        defaults: &AgentDefaults,
        max_context_tokens: usize,
        model_context_window: Option<usize>,
    ) -> AgentLoopImpl {
        AgentLoopImpl::new(AgentLoopConfig {
            provider: crate::interface::test_support::make_stub_provider(),
            tool_registry: Box::new(ToolRegistryImpl::new()),
            model: "stub".into(),
            max_tokens: 100,
            temperature: 0.0,
            spill_store: None,
            session_key: String::new(),
            context_collapse_after_tool_calls: u32::MAX,
            max_context_tokens,
            progress_callback: None,
            streaming: false,
            effort: None,
            audit_log: None,
            pin_recent_turns: defaults.pin_recent_turns,
            context_collapse_after_messages: defaults.context_collapse_after_messages,
            model_context_window,
        })
    }

    /// Four big turn-stamped assistant messages plus the in-flight prompt.
    fn oversized_history(big: &str) -> Vec<Message> {
        let mut v: Vec<Message> = (1..=4u32)
            .map(|t| {
                let mut m = Message::assistant(big, vec![]);
                m.turn = Some(t);
                m
            })
            .collect();
        v.push(Message::user("new prompt"));
        v
    }

    /// A config-file `pin_recent_turns` value must change loop behaviour
    /// when threaded through the constructor field (#1045).
    #[tokio::test]
    async fn config_pin_recent_turns_reaches_the_loop() {
        let defaults: AgentDefaults = serde_json::from_str(r#"{"pin_recent_turns": 3}"#).unwrap();
        assert_eq!(defaults.pin_recent_turns, 3, "config field parses");
        let big = "x".repeat(2000); // ~500 tokens each

        // Control: default configuration (pin 2) removes turn 2 in full form.
        let agent = agent_with(&AgentDefaults::default(), 100, None);
        let mut messages = oversized_history(&big);
        agent.process(&mut messages).await.unwrap();
        assert!(
            !messages
                .iter()
                .any(|m| m.turn == Some(2) && m.content == big),
            "control: with the default pin of 2, turn 2 must not survive in full"
        );

        // The configured pin of 3 keeps turn 2 in full despite the same budget.
        let agent = agent_with(&defaults, 100, None);
        let mut messages = oversized_history(&big);
        agent.process(&mut messages).await.unwrap();
        assert!(
            messages
                .iter()
                .any(|m| m.turn == Some(2) && m.content == big),
            "a configured pin_recent_turns=3 must change pinning in the loop"
        );
    }

    /// A config-file `context_collapse_after_messages` value must change
    /// loop behaviour through the constructor field (#1046 AC5).
    #[tokio::test]
    async fn config_message_collapse_threshold_reaches_the_loop() {
        let defaults: AgentDefaults =
            serde_json::from_str(r#"{"context_collapse_after_messages": 1}"#).unwrap();
        let mut old_a = Message::assistant("an old answer with plenty of words in it", vec![]);
        old_a.turn = Some(1);
        old_a.spill_id = Some("turn1:msg:assistant".to_string());
        let mut old_b = Message::assistant("another old answer with plenty of words", vec![]);
        old_b.turn = Some(2);
        old_b.spill_id = Some("turn2:msg:assistant".to_string());

        let agent = agent_with(&defaults, 190_000, None);
        let mut messages = vec![old_a, old_b, Message::user("new prompt")];
        agent.process(&mut messages).await.unwrap();
        assert!(
            messages
                .iter()
                .any(|m| m.is_collapsed && m.content.contains("recall(\"turn1:msg:assistant\")")),
            "a configured context_collapse_after_messages=1 must collapse the \
             oldest conversation message in the loop"
        );
    }

    /// The model registry's known window flows through the constructor
    /// field as the effective budget clamp (#1044 AC2).
    #[test]
    fn model_window_from_registry_clamps_the_effective_budget() {
        let window = crate::infrastructure::model_registry::ModelRegistry::builtin()
            .context_window_for("anthropic-api/claude-sonnet-5");
        assert_eq!(window, Some(1_000_000), "precondition: a known window");
        let agent = agent_with(&AgentDefaults::default(), 200_000, window);
        assert_eq!(
            agent.effective_max_context_tokens(),
            200_000,
            "config stays the override below the window"
        );
        let agent = agent_with(&AgentDefaults::default(), 2_000_000, window);
        assert_eq!(
            agent.effective_max_context_tokens(),
            1_000_000,
            "a smaller known window must clamp the configured budget"
        );
    }
}
