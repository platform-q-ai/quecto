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
    use quecto::interface::gateway::resolve_api_key;

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
    use quecto::interface::gateway::check_provider_readiness;

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
