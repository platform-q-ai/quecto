use super::*;

// Voice Steps
// ===========================================================================

#[given(expr = "a Groq Whisper client with api_key {string}")]
fn given_whisper_client_with_key(world: &mut QuectoWorld, api_key: String) {
    // Client will be reconfigured once the mock server is set up
    world.whisper_client = Some(GroqWhisperClient::new(&api_key));
}

#[given("a Groq Whisper client with no api_key")]
fn given_whisper_client_no_key(world: &mut QuectoWorld) {
    world.whisper_client = Some(GroqWhisperClient::new(""));
}

#[given(expr = "a mock Whisper API that returns transcription {string}")]
fn given_mock_whisper_success(world: &mut QuectoWorld, text: String) {
    // Use a single tokio runtime for mock server setup + keep it alive
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/openai/v1/audio/transcriptions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"text": text})),
            )
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });

    world.whisper_client = Some(GroqWhisperClient::with_base_url("gsk-test-key", &uri));
    world._wiremock_server_uri = Some(uri);
    // Leak both the runtime and server so the mock HTTP server stays alive
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given("a mock Whisper API that returns an error")]
fn given_mock_whisper_error(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/openai/v1/audio/transcriptions"))
            .respond_with(
                wiremock::ResponseTemplate::new(500).set_body_string("Internal Server Error"),
            )
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });

    world.whisper_client = Some(GroqWhisperClient::with_base_url("gsk-test-key", &uri));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[when("the whisper client transcribes audio")]
fn when_client_transcribes(world: &mut QuectoWorld) {
    let client = world
        .whisper_client
        .as_ref()
        .expect("whisper client not set");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result =
        rt.block_on(client.transcribe_bytes(b"fake audio data".to_vec(), "test_audio.ogg"));

    world.transcription_result = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the transcription result should be {string}")]
fn then_transcription_is(world: &mut QuectoWorld, expected: String) {
    let result = world
        .transcription_result
        .as_ref()
        .expect("no transcription result");
    match result {
        Ok(tr) => assert_eq!(
            tr.text, expected,
            "expected transcription '{}', got '{}'",
            expected, tr.text
        ),
        Err(e) => panic!("expected successful transcription, got error: {}", e),
    }
}

#[then(expr = "the transcription should fail with {string}")]
fn then_transcription_fails_with(world: &mut QuectoWorld, expected_msg: String) {
    let result = world
        .transcription_result
        .as_ref()
        .expect("no transcription result");
    match result {
        Ok(tr) => panic!(
            "expected transcription to fail with '{}', but got success: '{}'",
            expected_msg, tr.text
        ),
        Err(e) => assert!(
            e.contains(&expected_msg),
            "expected error containing '{}', got: {}",
            expected_msg,
            e
        ),
    }
}

#[then("the transcription should fail with an error message")]
fn then_transcription_fails_with_any(world: &mut QuectoWorld) {
    let result = world
        .transcription_result
        .as_ref()
        .expect("no transcription result");
    assert!(
        result.is_err(),
        "expected transcription to fail, but got success: {:?}",
        result
    );
}

// ===========================================================================
// Observability Steps
// ===========================================================================

#[given("a valid config with OpenAI API key set")]
fn given_valid_config_with_openai(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test-key-123" }
        }
    }"#;
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
}

#[given("a config with OpenAI api_key set and Anthropic not set")]
fn given_config_openai_set_anthropic_not(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test-key-456" },
            "anthropic": { "api_key": "" }
        }
    }"#;
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
}

#[given(expr = "a config with OpenAI api_key {string} set")]
fn given_config_with_specific_openai_key(world: &mut QuectoWorld, api_key: String) {
    ensure_temp_dir(world);
    let config_json = format!(
        r#"{{
        "providers": {{
            "openai": {{ "api_key": "{}" }}
        }}
    }}"#,
        api_key
    );
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
}

#[then(expr = "the output should not contain {string}")]
fn then_output_should_not_contain(world: &mut QuectoWorld, unexpected: String) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    assert!(
        !combined.contains(&unexpected),
        "expected output NOT to contain '{}', but got:\nstdout: {}\nstderr: {}",
        unexpected,
        world.stdout,
        world.stderr
    );
}

// ===========================================================================
