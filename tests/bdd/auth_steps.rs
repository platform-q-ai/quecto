use super::*;

// Auth Steps
// ===========================================================================

fn ensure_credential_store(world: &mut QuectoWorld) {
    if world.credential_store.is_none() {
        if world._temp_dir.is_none() {
            let td = TempDir::new().expect("failed to create temp dir");
            world._temp_dir = Some(td);
        }
        let base = world._temp_dir.as_ref().unwrap().path().to_path_buf();
        world.credential_store = Some(CredentialStore::new(base));
    }
}

#[given("a credential store")]
fn given_credential_store(world: &mut QuectoWorld) {
    ensure_credential_store(world);
}

#[given("a credential store with no credentials")]
fn given_credential_store_empty(world: &mut QuectoWorld) {
    ensure_credential_store(world);
}

#[given(expr = "a stored credential for {string} with method {string}")]
fn given_stored_credential(world: &mut QuectoWorld, provider: String, method: String) {
    ensure_credential_store(world);
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    let auth_method = match method.as_str() {
        "oauth" => AuthMethod::OAuth,
        _ => AuthMethod::Token,
    };
    store
        .store(Credential {
            provider,
            token: "test-token".to_string(),
            method: auth_method,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
}

#[given(expr = "a stored credential for {string} that is expired")]
fn given_expired_credential(world: &mut QuectoWorld, provider: String) {
    ensure_credential_store(world);
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    store
        .store(Credential {
            provider,
            token: "expired-token".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0), // epoch — always expired
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
}

#[when(expr = "I store a token {string} for provider {string}")]
fn when_store_token(world: &mut QuectoWorld, token: String, provider: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    store
        .store(Credential {
            provider,
            token,
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
}

#[when("I check auth status")]
fn when_check_auth_status(world: &mut QuectoWorld) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    world.auth_status = Some(store.status_summary().unwrap());
}

#[when(expr = "I remove the credential for {string}")]
fn when_remove_credential(world: &mut QuectoWorld, provider: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    store.remove(&provider).unwrap();
}

#[when("I remove all credentials")]
fn when_remove_all_credentials(world: &mut QuectoWorld) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    store.remove_all().unwrap();
}

#[then(expr = "the credential for {string} should exist")]
fn then_credential_exists(world: &mut QuectoWorld, provider: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    assert!(
        store.exists(&provider).unwrap(),
        "credential for '{}' should exist",
        provider
    );
}

#[then(expr = "the credential for {string} should not exist")]
fn then_credential_not_exists(world: &mut QuectoWorld, provider: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    assert!(
        !store.exists(&provider).unwrap(),
        "credential for '{}' should not exist",
        provider
    );
}

#[then(expr = "the credential token should be {string}")]
fn then_credential_token_is(world: &mut QuectoWorld, expected: String) {
    let store = world
        .credential_store
        .as_ref()
        .expect("credential store not set");
    // Get the most recently stored credential (from the last store operation)
    let list = store.list().unwrap();
    let cred = list.first().expect("no credentials found");
    assert_eq!(
        cred.token, expected,
        "expected token '{}', got '{}'",
        expected, cred.token
    );
}

#[then("the auth status should report no providers")]
fn then_auth_status_no_providers(world: &mut QuectoWorld) {
    let status = world.auth_status.as_ref().expect("no auth status");
    assert!(status.is_empty(), "expected no providers, got {:?}", status);
}

#[then(expr = "the auth status should include {string}")]
fn then_auth_status_includes(world: &mut QuectoWorld, provider: String) {
    let status = world.auth_status.as_ref().expect("no auth status");
    assert!(
        status.iter().any(|s| s.provider == provider),
        "expected auth status to include '{}', got: {:?}",
        provider,
        status.iter().map(|s| &s.provider).collect::<Vec<_>>()
    );
}

#[then(expr = "the auth status for {string} should be {string}")]
fn then_auth_status_for_provider(
    world: &mut QuectoWorld,
    provider: String,
    expected_status: String,
) {
    let status = world.auth_status.as_ref().expect("no auth status");
    let entry = status
        .iter()
        .find(|s| s.provider == provider)
        .unwrap_or_else(|| panic!("no auth status for provider '{}'", provider));
    assert_eq!(
        entry.status, expected_status,
        "expected status '{}' for '{}', got '{}'",
        expected_status, provider, entry.status
    );
}

// ===========================================================================
// Auth CLI Steps
// ===========================================================================

#[given("a quecto base directory at a temporary path")]
fn given_quecto_base_dir_temp(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
}

#[then(expr = "the credential for {string} should exist in the base directory")]
fn then_credential_exists_in_base(world: &mut QuectoWorld, provider: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    assert!(
        store.exists(&provider).unwrap(),
        "credential for '{}' should exist in base directory {}",
        provider,
        base.display()
    );
}

#[then(expr = "the credential for {string} should not exist in the base directory")]
fn then_credential_not_exists_in_base(world: &mut QuectoWorld, provider: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    assert!(
        !store.exists(&provider).unwrap(),
        "credential for '{}' should not exist in base directory {}",
        provider,
        base.display()
    );
}

#[given(expr = "a stored credential for {string} in the base directory")]
fn given_stored_credential_in_base(world: &mut QuectoWorld, provider: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider,
            token: "test-token".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
}

#[given(expr = "a stored credential for {string} with method {string} in the base directory")]
fn given_stored_credential_method_in_base(
    world: &mut QuectoWorld,
    provider: String,
    method: String,
) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let auth_method = match method.as_str() {
        "oauth" => AuthMethod::OAuth,
        _ => AuthMethod::Token,
    };
    store
        .store(Credential {
            provider,
            token: "test-token".to_string(),
            method: auth_method,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
}

#[given(expr = "a stored credential for {string} that is expired in the base directory")]
fn given_expired_credential_in_base(world: &mut QuectoWorld, provider: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider,
            token: "expired-token".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0), // epoch — always expired
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
}

// ===========================================================================
// Auth Gateway Wiring Steps
// ===========================================================================

#[given(expr = "a config with no API key for {string}")]
fn given_config_no_api_key(world: &mut QuectoWorld, _provider: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let config: Config = serde_json::from_str("{}").unwrap();
    // Write config to base for gateway to load
    let config_json = serde_json::to_string_pretty(&config).unwrap();
    std::fs::write(base.join("config.json"), config_json).unwrap();
    world.gateway_config = Some(config);
    world.gateway_credential_store = Some(CredentialStore::new(&base));
}

#[given(expr = "a config with API key {string} for {string}")]
fn given_config_with_api_key(world: &mut QuectoWorld, api_key: String, provider: String) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let config_json = match provider.as_str() {
        "openai" => format!(
            r#"{{"providers": {{"openai": {{"api_key": "{}"}}}}}}"#,
            api_key
        ),
        "anthropic" => format!(
            r#"{{"providers": {{"anthropic": {{"api_key": "{}"}}}}}}"#,
            api_key
        ),
        _ => "{}".to_string(),
    };
    let config: Config = serde_json::from_str(&config_json).unwrap();
    std::fs::write(
        base.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();
    world.gateway_config = Some(config);
    world.gateway_credential_store = Some(CredentialStore::new(&base));
}

#[given(expr = "a stored credential for {string} with token {string}")]
fn given_stored_credential_with_token(world: &mut QuectoWorld, provider: String, token: String) {
    // If gateway_credential_store is set, use it; otherwise use the default credential_store
    if let Some(ref store) = world.gateway_credential_store {
        store
            .store(Credential {
                provider,
                token,
                method: AuthMethod::Token,
                expires_at: None,
                refresh_token: None,
                account_id: None,
            })
            .unwrap();
    } else {
        ensure_credential_store(world);
        let store = world.credential_store.as_ref().unwrap();
        store
            .store(Credential {
                provider,
                token,
                method: AuthMethod::Token,
                expires_at: None,
                refresh_token: None,
                account_id: None,
            })
            .unwrap();
    }
}

#[given(expr = "no stored credential for {string}")]
fn given_no_stored_credential(world: &mut QuectoWorld, provider: String) {
    // Ensure the store exists but has no credential for this provider
    if let Some(ref store) = world.gateway_credential_store {
        let _ = store.remove(&provider);
    }
}

#[when("the gateway initializes providers")]
fn when_gateway_initializes_providers(world: &mut QuectoWorld) {
    use quecto::interface::shared::resolve_api_key;

    let config = world
        .gateway_config
        .as_ref()
        .expect("gateway config not set");
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let creds = store.load_snapshot().unwrap_or_default();

    let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
    world.gateway_resolved_api_key = Some(resolved);
    world.gateway_cred_snapshot = Some(creds);
}

#[then(expr = "the OpenAI provider should use API key {string}")]
fn then_openai_provider_uses_key(world: &mut QuectoWorld, expected: String) {
    let actual = world
        .gateway_resolved_api_key
        .as_ref()
        .expect("no resolved API key");
    assert_eq!(
        actual, &expected,
        "expected OpenAI API key '{}', got '{}'",
        expected, actual
    );
}

#[when("the gateway checks provider readiness")]
fn when_gateway_checks_readiness(world: &mut QuectoWorld) {
    use quecto::interface::shared::check_provider_readiness;

    let store = world
        .gateway_credential_store
        .as_ref()
        .or(world.credential_store.as_ref())
        .expect("no credential store set");
    let creds = store.load_snapshot().unwrap_or_default();
    let needs_reauth = check_provider_readiness(&creds);
    world.gateway_readiness_report = Some(needs_reauth);
}

#[then(expr = "the gateway should report {string} needs re-authentication")]
fn then_gateway_reports_reauth(world: &mut QuectoWorld, provider: String) {
    let report = world
        .gateway_readiness_report
        .as_ref()
        .expect("no readiness report");
    assert!(
        report.contains(&provider),
        "expected '{}' to need re-authentication, got: {:?}",
        provider,
        report
    );
}

// ===========================================================================
// Interactive Auth + OAuth Steps
// ===========================================================================

#[when(expr = "I start quecto with arguments {string}")]
fn when_start_quecto_with_args(world: &mut QuectoWorld, args_str: String) {
    // Store the args for deferred execution (next step will provide stdin).
    let mut args = vec!["quecto".to_string()];
    args.extend(shell_split(&args_str));
    world.pending_cli_args = Some(args);
}

#[when(expr = "I paste the token {string}")]
fn when_paste_token(world: &mut QuectoWorld, token: String) {
    let args = world
        .pending_cli_args
        .take()
        .expect("no pending CLI args — call 'I start quecto' first");
    // Set stdin_data on the CLI context so cmd_auth_login reads from it.
    world.cli_context.stdin_data = Some(token);
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[given(expr = "a mock OAuth server for {string}")]
fn given_mock_oauth_server(world: &mut QuectoWorld, _provider: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let uri = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let uri = server.uri();
        let _leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        uri
    });
    std::mem::forget(rt);
    world.cli_context.oauth_base_url = Some(uri);
}

#[given(expr = "a mock OAuth server for {string} with token exchange")]
fn given_mock_oauth_server_with_token_exchange(world: &mut QuectoWorld, _provider: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let uri = rt.block_on(async {
        let server = wiremock::MockServer::start().await;

        let token_response = serde_json::json!({
            "access_token": "mock-access-token-xyz",
            "refresh_token": "mock-refresh-token-xyz",
            "expires_in": 3600
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth/token"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&token_response))
            .mount(&server)
            .await;

        let uri = server.uri();
        let _leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        uri
    });
    std::mem::forget(rt);
    world.cli_context.oauth_base_url = Some(uri);
}

#[given(expr = "a mock OAuth server for {string} supporting device code flow")]
fn given_mock_oauth_device_code(world: &mut QuectoWorld, _provider: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, _server_ref) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;

        let response = serde_json::json!({
            "device_code": "DEVCODE-TEST-123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://auth.example.com/device"
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/device/code"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let uri = server.uri();
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        (uri, leaked)
    });
    std::mem::forget(rt);
    world.cli_context.oauth_base_url = Some(uri);
}

#[then("the output should contain a URL to open in the browser")]
fn then_output_contains_url(world: &mut QuectoWorld) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    assert!(
        combined.contains("http://") || combined.contains("https://"),
        "expected output to contain a URL, got:\n{}",
        combined
    );
}

#[then("the output should contain a device code URL")]
fn then_output_contains_device_code_url(world: &mut QuectoWorld) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    assert!(
        combined.contains("https://") || combined.contains("http://"),
        "expected output to contain a device code URL, got:\n{}",
        combined
    );
    assert!(
        combined.contains("Go to:"),
        "expected 'Go to:' prefix for device code URL, got:\n{}",
        combined
    );
}

#[then("the output should contain a user code to enter")]
fn then_output_contains_user_code(world: &mut QuectoWorld) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    assert!(
        combined.contains("Enter code:"),
        "expected output to contain 'Enter code:', got:\n{}",
        combined
    );
}

#[then(expr = "the stored credential method should be {string}")]
fn then_stored_credential_method(world: &mut QuectoWorld, expected_method: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let list = store.list().unwrap();
    assert!(!list.is_empty(), "expected at least one stored credential");
    let cred = list.first().unwrap();
    assert_eq!(
        cred.method.as_str(),
        expected_method,
        "expected credential method '{}', got '{}'",
        expected_method,
        cred.method.as_str()
    );
}

// ===========================================================================
// Gateway OAuth Token Refresh Steps (issue #254)
// ===========================================================================

#[given(expr = "a stored expired OAuth credential for {string} with refresh token {string}")]
fn given_expired_oauth_credential(
    world: &mut QuectoWorld,
    provider: String,
    refresh_token: String,
) {
    // Ensure temp dir and gateway credential store
    if world.gateway_credential_store.is_none() {
        ensure_temp_dir(world);
        let base = base_path(world);
        world.gateway_credential_store = Some(CredentialStore::new(&base));
        if world.gateway_config.is_none() {
            let config: Config = serde_json::from_str("{}").unwrap();
            world.gateway_config = Some(config);
        }
    }
    let store = world
        .gateway_credential_store
        .as_ref()
        .expect("gateway credential store not set");
    store
        .store(Credential {
            provider,
            token: "sk-expired-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0), // always expired
            refresh_token: Some(refresh_token),
            account_id: None,
        })
        .unwrap();
}

#[given(expr = "a stored valid OAuth credential for {string} with token {string}")]
fn given_valid_oauth_credential(world: &mut QuectoWorld, provider: String, token: String) {
    if world.gateway_credential_store.is_none() {
        ensure_temp_dir(world);
        let base = base_path(world);
        world.gateway_credential_store = Some(CredentialStore::new(&base));
        if world.gateway_config.is_none() {
            let config: Config = serde_json::from_str("{}").unwrap();
            world.gateway_config = Some(config);
        }
    }
    let store = world
        .gateway_credential_store
        .as_ref()
        .expect("gateway credential store not set");
    store
        .store(Credential {
            provider,
            token,
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX), // far future — never expired
            refresh_token: Some("rt-unused".to_string()),
            account_id: None,
        })
        .unwrap();
}

#[given(expr = "a mock OAuth refresh server that returns a new token {string}")]
fn given_mock_oauth_refresh_server_success(world: &mut QuectoWorld, new_token: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, leaked) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;

        let response = serde_json::json!({
            "access_token": new_token,
            "refresh_token": "rt-new-refresh",
            "expires_in": 28800
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth/token"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let uri = server.uri();
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        (uri, leaked)
    });
    std::mem::forget(rt);
    world.gateway_oauth_mock_uri = Some(uri);
    world._gateway_oauth_mock_server = Some(leaked);
}

#[given("a mock OAuth refresh server that returns an error")]
fn given_mock_oauth_refresh_server_error(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, leaked) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth/token"))
            .respond_with(wiremock::ResponseTemplate::new(400).set_body_string("invalid_grant"))
            .mount(&server)
            .await;

        let uri = server.uri();
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        (uri, leaked)
    });
    std::mem::forget(rt);
    world.gateway_oauth_mock_uri = Some(uri);
    world._gateway_oauth_mock_server = Some(leaked);
}

#[when(expr = "the gateway resolves API key with refresh for {string}")]
fn when_gateway_resolves_with_refresh(world: &mut QuectoWorld, provider: String) {
    use quecto::infrastructure::auth::oauth::OAuthConfig;
    use quecto::interface::shared::resolve_api_key_with_refresh_async_with_oauth_config;

    let config = world
        .gateway_config
        .as_ref()
        .expect("gateway config not set");
    let base = base_path(world);
    let store = CredentialStore::new(&base);

    let config_key = match provider.as_str() {
        "openai" => &config.providers.openai.api_key,
        "anthropic" => &config.providers.anthropic.api_key,
        _ => panic!("unknown provider: {}", provider),
    };

    let oauth_config = world
        .gateway_oauth_mock_uri
        .as_ref()
        .map(|uri| OAuthConfig::with_base_url(uri))
        .unwrap_or_else(|| {
            OAuthConfig::for_provider(&provider)
                .unwrap_or_else(|| OAuthConfig::with_base_url("http://localhost:0"))
        });

    let rt = tokio::runtime::Runtime::new().unwrap();
    let resolved = rt.block_on(resolve_api_key_with_refresh_async_with_oauth_config(
        config_key,
        &store,
        &provider,
        &oauth_config,
    ));

    world.gateway_resolved_api_key = Some(resolved);
}

#[then(expr = "the persisted credential for {string} should have token {string}")]
fn then_persisted_credential_has_token(
    world: &mut QuectoWorld,
    provider: String,
    expected_token: String,
) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let creds = store.load_snapshot().unwrap();
    let cred = creds
        .get(&provider)
        .unwrap_or_else(|| panic!("no credential found for provider '{}'", provider));
    assert_eq!(
        cred.token, expected_token,
        "expected persisted token '{}' for '{}', got '{}'",
        expected_token, provider, cred.token
    );
}

#[then(expr = "the resolved API key should be {string}")]
fn then_resolved_api_key_is(world: &mut QuectoWorld, expected: String) {
    let actual = world
        .gateway_resolved_api_key
        .as_ref()
        .expect("no resolved API key");
    assert_eq!(
        actual, &expected,
        "expected resolved API key '{}', got '{}'",
        expected, actual
    );
}

// ===========================================================================
// Optional refresh_token in OAuth response steps (issue #257)
// ===========================================================================

#[given(expr = "a mock OAuth refresh server that omits refresh_token and returns token {string}")]
fn given_mock_oauth_refresh_server_no_refresh_token(world: &mut QuectoWorld, new_token: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, leaked) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;

        // Response deliberately omits refresh_token (valid per RFC 6749 §5.1)
        let response = serde_json::json!({
            "access_token": new_token,
            "expires_in": 28800
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth/token"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let uri = server.uri();
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        (uri, leaked)
    });
    std::mem::forget(rt);
    world.gateway_oauth_mock_uri = Some(uri);
    world._gateway_oauth_mock_server = Some(leaked);
}

#[given(
    expr = "a mock OAuth refresh server that returns a new token {string} with refresh token {string}"
)]
fn given_mock_oauth_refresh_server_with_refresh_token(
    world: &mut QuectoWorld,
    new_token: String,
    new_refresh: String,
) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, leaked) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;

        let response = serde_json::json!({
            "access_token": new_token,
            "refresh_token": new_refresh,
            "expires_in": 28800
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth/token"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let uri = server.uri();
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        (uri, leaked)
    });
    std::mem::forget(rt);
    world.gateway_oauth_mock_uri = Some(uri);
    world._gateway_oauth_mock_server = Some(leaked);
}

#[then(expr = "the persisted credential for {string} should have refresh token {string}")]
fn then_persisted_credential_has_refresh_token(
    world: &mut QuectoWorld,
    provider: String,
    expected_refresh: String,
) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let creds = store.load_snapshot().unwrap();
    let cred = creds
        .get(&provider)
        .unwrap_or_else(|| panic!("no credential found for provider '{}'", provider));
    assert_eq!(
        cred.refresh_token.as_deref(),
        Some(expected_refresh.as_str()),
        "expected persisted refresh token '{}' for '{}', got '{:?}'",
        expected_refresh,
        provider,
        cred.refresh_token
    );
}

#[given("a mock OAuth token exchange server that omits refresh_token")]
fn given_mock_oauth_token_exchange_no_refresh_token(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, leaked) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;

        let response = serde_json::json!({
            "access_token": "sk-ant-oat01-exchanged",
            "expires_in": 28800
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth/token"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let uri = server.uri();
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        (uri, leaked)
    });
    std::mem::forget(rt);
    world.gateway_oauth_mock_uri = Some(uri);
    world._gateway_oauth_mock_server = Some(leaked);
}

#[when("an OAuth token exchange is performed")]
fn when_oauth_token_exchange(world: &mut QuectoWorld) {
    use quecto::infrastructure::auth::oauth::{OAuthConfig, exchange_anthropic_code};

    let uri = world
        .gateway_oauth_mock_uri
        .as_ref()
        .expect("mock OAuth URI not set");
    let config = OAuthConfig::with_base_url(uri);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(exchange_anthropic_code(
        &config,
        "code123#state456",
        "verifier",
    ));
    world.gateway_token_exchange_result = Some(result);
}

#[then(expr = "the token exchange should succeed with access token {string}")]
fn then_token_exchange_succeeds(world: &mut QuectoWorld, expected_token: String) {
    let result = world
        .gateway_token_exchange_result
        .as_ref()
        .expect("no token exchange result");
    let resp = result.as_ref().expect("token exchange failed");
    assert_eq!(
        resp.access_token, expected_token,
        "expected access token '{}', got '{}'",
        expected_token, resp.access_token
    );
}

#[then("the token exchange response should have no refresh token")]
fn then_token_exchange_no_refresh_token(world: &mut QuectoWorld) {
    let result = world
        .gateway_token_exchange_result
        .as_ref()
        .expect("no token exchange result");
    let resp = result.as_ref().expect("token exchange failed");
    assert_eq!(
        resp.refresh_token, None,
        "expected no refresh token, got '{:?}'",
        resp.refresh_token
    );
}

// ===========================================================================
// OpenAI OAuth import refresh steps (issue #258)
// ===========================================================================

#[given("an opencode auth.json with expired OpenAI OAuth credential")]
fn given_opencode_expired_openai(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let now = quecto::infrastructure::time::unix_timestamp_secs();
    // Token expired 100 seconds ago
    let expires_ms = (now - 100) * 1000;
    world.gateway_import_auth_json = Some(serde_json::json!({
        "openai": {
            "type": "oauth",
            "access": "eyJ-old-expired",
            "refresh": "rt-old-openai",
            "expires": expires_ms
        }
    }));
}

#[given(expr = "an opencode auth.json with valid OpenAI OAuth credential {string}")]
fn given_opencode_valid_openai(world: &mut QuectoWorld, token: String) {
    ensure_temp_dir(world);
    let now = quecto::infrastructure::time::unix_timestamp_secs();
    let expires_ms = (now + 7200) * 1000;
    world.gateway_import_auth_json = Some(serde_json::json!({
        "openai": {
            "type": "oauth",
            "access": token,
            "refresh": "rt-valid",
            "expires": expires_ms
        }
    }));
}

#[when("the opencode credentials are imported")]
fn when_opencode_imported(world: &mut QuectoWorld) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let auth_json = world
        .gateway_import_auth_json
        .as_ref()
        .expect("auth.json not set");

    let oauth_base_url = world.gateway_oauth_mock_uri.clone();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut stdout = String::new();
    let mut stderr = String::new();
    let params = quecto::interface::cli::OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: oauth_base_url.as_deref(),
    };
    quecto::interface::cli::auth_import_openai(auth_json, &params, &mut stdout, &mut stderr);
    world.gateway_import_stdout = Some(stdout);
    world.gateway_import_stderr = Some(stderr);
}

#[then(expr = "the stored OpenAI credential should have token {string}")]
fn then_stored_openai_token(world: &mut QuectoWorld, expected_token: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let creds = store.load_snapshot().unwrap();
    let cred = creds.get("openai").expect("no OpenAI credential found");
    assert_eq!(
        cred.token, expected_token,
        "expected OpenAI token '{}', got '{}'",
        expected_token, cred.token
    );
}

#[then(expr = "the import output should contain {string}")]
fn then_import_output_contains(world: &mut QuectoWorld, expected: String) {
    let stdout = world.gateway_import_stdout.as_deref().unwrap_or("");
    let stderr = world.gateway_import_stderr.as_deref().unwrap_or("");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains(&expected),
        "expected import output to contain '{}', got stdout='{}' stderr='{}'",
        expected,
        stdout,
        stderr
    );
}

// ===========================================================================
// Consistent expires_at safety margin steps (issue #256)
// ===========================================================================

#[given(expr = "an OAuth token with expires_in of {int} seconds")]
fn given_oauth_token_expires_in(world: &mut QuectoWorld, expires_in: u64) {
    world.gateway_expires_in = Some(expires_in);
}

#[when("expires_at_with_margin is calculated")]
fn when_expires_at_with_margin(world: &mut QuectoWorld) {
    let expires_in = world.gateway_expires_in.expect("expires_in not set");
    let result = quecto::interface::shared::expires_at_with_margin(expires_in);
    world.gateway_computed_expires_at = Some(result);
}

#[then(expr = "the resulting expires_at should be {int} seconds from now")]
fn then_expires_at_is_seconds_from_now(world: &mut QuectoWorld, expected_offset: i64) {
    let now = quecto::infrastructure::time::unix_timestamp_secs();
    let actual = world
        .gateway_computed_expires_at
        .expect("expires_at not computed");
    let expected = now + expected_offset;
    assert!(
        (actual - expected).abs() <= 2,
        "expected expires_at ~{} ({} seconds from now), got {} (diff: {}s)",
        expected,
        expected_offset,
        actual,
        (actual - expected).abs()
    );
}

#[then(
    expr = "the persisted credential for {string} should have expires_at with 300-second safety margin for {int} seconds"
)]
fn then_persisted_credential_has_margin(
    world: &mut QuectoWorld,
    provider: String,
    expires_in: i64,
) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    let creds = store.load_snapshot().unwrap();
    let cred = creds
        .get(&provider)
        .unwrap_or_else(|| panic!("no credential found for provider '{}'", provider));

    let now = quecto::infrastructure::time::unix_timestamp_secs();
    let expected_with_margin = now + expires_in - 300;
    let actual = cred.expires_at.expect("expires_at not set");

    assert!(
        (actual - expected_with_margin).abs() <= 2,
        "expected expires_at ~{} (now + {} - 300), got {} (diff: {}s). \
         Without margin would be ~{} — if that matches, the margin is missing.",
        expected_with_margin,
        expires_in,
        actual,
        (actual - expected_with_margin).abs(),
        now + expires_in
    );
}

// --- RefreshableProvider BDD steps (issue #255) ---

use quecto::infrastructure::providers::refreshable::{
    ProviderFactory, RefreshFn, RefreshableConfig, RefreshableProvider,
};

/// Helper: build a mock refresh function that stores a new token.
fn make_bdd_refresh(new_token: String) -> RefreshFn {
    Arc::new(move |store, provider_name| {
        let token = new_token.clone();
        let provider_name = provider_name.to_string();
        let store = store.clone();
        Box::pin(async move {
            store
                .store(Credential {
                    provider: provider_name,
                    token: token.clone(),
                    method: AuthMethod::OAuth,
                    expires_at: Some(i64::MAX),
                    refresh_token: Some("rt-refreshed".to_string()),
                    account_id: None,
                })
                .map_err(|e| DomainError::Provider(format!("store error: {}", e)))?;
            Ok(token)
        })
    })
}

/// Mock provider that fails N times with 401, then succeeds.
#[derive(Debug)]
struct BddMockRetryProvider {
    call_count: Arc<std::sync::atomic::AtomicU32>,
    fail_until: u32,
}

impl BddMockRetryProvider {
    fn new(fail_until: u32) -> Self {
        Self {
            call_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            fail_until,
        }
    }
}

impl LlmProvider for BddMockRetryProvider {
    fn name(&self) -> &str {
        "bdd-mock"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            if count < self.fail_until {
                Err(DomainError::Provider(
                    "provider error (401): unauthorized".to_string(),
                ))
            } else {
                Ok(LlmResponse {
                    content: Some("refreshed-success".to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                })
            }
        })
    }
}

/// Mock provider that always returns 500.
#[derive(Debug)]
struct BddMock500Provider;

impl LlmProvider for BddMock500Provider {
    fn name(&self) -> &str {
        "bdd-mock-500"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Err(DomainError::Provider(
                "provider error (500): internal server error".to_string(),
            ))
        })
    }
}

/// Mock provider that always succeeds.
#[derive(Debug)]
struct BddMockSuccessProvider;

impl LlmProvider for BddMockSuccessProvider {
    fn name(&self) -> &str {
        "bdd-mock-ok"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("normal-success".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
            })
        })
    }
}

#[given("an OAuth-backed provider that returns 401 on first call")]
fn given_provider_401_first(world: &mut QuectoWorld) {
    ensure_credential_store(world);
    let store = world.credential_store.as_ref().unwrap();
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

    let base_dir = store.path().parent().unwrap();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let inner = Arc::new(BddMockRetryProvider::new(1));
    let cc = call_count.clone();
    let factory: ProviderFactory = Arc::new(move |_| {
        Arc::new(BddMockRetryProvider {
            call_count: cc.clone(),
            fail_until: 0, // rebuilt provider always succeeds
        }) as Arc<dyn LlmProvider>
    });
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store: store_arc,
        provider_name: "anthropic".to_string(),
        refresh_fn: make_bdd_refresh("sk-ant-oat01-fresh".to_string()),
        factory,
    });
    world.provider = Some(Arc::new(refreshable));
}

#[given(expr = "the provider returns success after token refresh")]
fn given_provider_succeeds_after_refresh(_world: &mut QuectoWorld) {
    // Already configured by the retry provider (fail_until=1)
}

#[given("an OAuth-backed provider that returns 500")]
fn given_provider_500(world: &mut QuectoWorld) {
    ensure_credential_store(world);
    let store = world.credential_store.as_ref().unwrap();
    let base_dir = store.path().parent().unwrap();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let inner = Arc::new(BddMock500Provider);
    let factory: ProviderFactory =
        Arc::new(|_| Arc::new(BddMock500Provider) as Arc<dyn LlmProvider>);
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store: store_arc,
        provider_name: "anthropic".to_string(),
        refresh_fn: make_bdd_refresh("unused".to_string()),
        factory,
    });
    world.provider = Some(Arc::new(refreshable));
}

#[given("an OAuth-backed provider that returns success")]
fn given_provider_success(world: &mut QuectoWorld) {
    ensure_credential_store(world);
    let store = world.credential_store.as_ref().unwrap();
    let base_dir = store.path().parent().unwrap();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let inner = Arc::new(BddMockSuccessProvider);
    let factory: ProviderFactory =
        Arc::new(|_| Arc::new(BddMockSuccessProvider) as Arc<dyn LlmProvider>);
    let refreshable = RefreshableProvider::new(RefreshableConfig {
        inner,
        store: store_arc,
        provider_name: "anthropic".to_string(),
        refresh_fn: make_bdd_refresh("unused".to_string()),
        factory,
    });
    world.provider = Some(Arc::new(refreshable));
}

#[when("a chat request is sent through the refreshable provider")]
async fn when_chat_through_refreshable(world: &mut QuectoWorld) {
    let provider = world.provider.as_ref().expect("provider not set");
    let request = ChatRequest {
        messages: &[],
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
    };
    world.refreshable_result = Some(provider.chat(request).await);
}

#[then("the request should succeed with the refreshed token")]
fn then_succeed_with_refreshed(world: &mut QuectoWorld) {
    let result = world.refreshable_result.as_ref().expect("no result");
    let resp = result.as_ref().expect("expected Ok, got Err");
    assert_eq!(
        resp.content.as_deref(),
        Some("refreshed-success"),
        "response should come from the retried call"
    );
}

#[then("the credential store should contain the refreshed token")]
fn then_store_has_refreshed_token(world: &mut QuectoWorld) {
    let store = world
        .credential_store
        .as_ref()
        .expect("no credential store");
    let creds = store.load_snapshot().unwrap();
    let cred = creds.get("anthropic").expect("no anthropic credential");
    assert_eq!(cred.token, "sk-ant-oat01-fresh");
}

#[then("the request should fail with a server error")]
fn then_fail_with_server_error(world: &mut QuectoWorld) {
    let result = world.refreshable_result.as_ref().expect("no result");
    let err = result.as_ref().expect_err("expected Err, got Ok");
    let msg = err.to_string();
    assert!(msg.contains("500"), "expected 500 error, got: {}", msg);
}

#[then("the request should succeed normally")]
fn then_succeed_normally(world: &mut QuectoWorld) {
    let result = world.refreshable_result.as_ref().expect("no result");
    let resp = result.as_ref().expect("expected Ok, got Err");
    assert_eq!(
        resp.content.as_deref(),
        Some("normal-success"),
        "response should be normal success"
    );
}
