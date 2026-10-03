use super::*;

// =======================================================================
// #931: classified, actionable terminal errors (enhance_provider_error)
// =======================================================================
//
// Acceptance criterion 3: after retries are exhausted (or for a
// non-retryable error) the returned error names the class and a
// remediation, not a raw provider string. Today `enhance_provider_error`
// only augments context/output-limit errors, so these assertions FAIL.

fn enhanced(message: &str) -> String {
    match enhance_provider_error(DomainError::Provider(message.to_string())) {
        DomainError::Provider(m) => m,
        other => panic!("expected Provider error, got {:?}", other),
    }
}

#[test]
fn test_enhance_rate_limit_error_adds_classified_guidance() {
    let input = "provider error (429): rate limit exceeded";
    let out = enhanced(input);
    // Discriminating: the appended guidance — not the raw input — must add the
    // class name and remediation. "throttled" never appears in the raw string.
    assert!(
        out.len() > input.len(),
        "enhancement must append guidance: {out}"
    );
    let lowered = out.to_ascii_lowercase();
    assert!(
        lowered.contains("throttled"),
        "appended guidance should name the rate-limit class: {out}"
    );
    assert!(
        lowered.contains("retry") || lowered.contains("later") || lowered.contains("frequency"),
        "terminal rate-limit error should include actionable remediation: {out}"
    );
}

#[test]
fn test_enhance_server_overload_error_adds_classified_guidance() {
    let input =
        "HTTP 529 from Anthropic: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}";
    let out = enhanced(input);
    assert!(
        out.len() > input.len(),
        "enhancement must append guidance: {out}"
    );
    let lowered = out.to_ascii_lowercase();
    assert!(
        lowered.contains("server/overload"),
        "appended guidance should name the server/overload class: {out}"
    );
    assert!(
        lowered.contains("retried") && lowered.contains("retry later"),
        "terminal server error should include actionable remediation: {out}"
    );
}

#[test]
fn test_enhance_network_error_adds_classified_guidance() {
    let input = "connection reset by peer";
    let out = enhanced(input);
    assert!(
        out.len() > input.len(),
        "enhancement must append guidance: {out}"
    );
    let lowered = out.to_ascii_lowercase();
    assert!(
        lowered.contains("connectivity"),
        "appended guidance should name the network remediation: {out}"
    );
    assert!(
        lowered.contains("retry") && lowered.contains("could not reach"),
        "terminal network error should include actionable remediation: {out}"
    );
}

#[test]
fn test_enhance_client_error_is_passed_through_without_retry_advice() {
    // A 4xx is the model's fault (e.g. malformed request); it must be passed
    // through verbatim — no class guidance, no "retry later" dressing.
    let input = "provider error (400): invalid_request_error";
    let out = enhanced(input);
    assert_eq!(
        out, input,
        "client 4xx must be passed through unchanged (no enhancement): {out}"
    );
}

// =======================================================================
// #931: a model-malformed tool call is always addressable, never fatal
// =======================================================================
//
// Acceptance criterion 2: when the provider rejects a model-emitted tool
// call as malformed (e.g. 400 invalid_request on the tool_use turn), the
// turn must not die — the model gets addressable `is_error` feedback and a
// chance to self-correct on the next turn. Today the agent loop returns
// `Err` for any Client error, so this FAILS.

#[tokio::test]
async fn test_malformed_tool_call_api_rejection_is_addressable_not_fatal() {
    // First provider call rejects a malformed tool call with a 400; the
    // second call (after the model is told to fix it) succeeds.
    let responses = vec![
        Err(DomainError::Provider(
            "provider error (400): invalid_request_error: tool_use input is malformed".to_string(),
        )),
        Ok(text_response("fixed it")),
    ];
    let provider = Arc::new(MockProvider::new_results(responses));
    let registry = MockRegistry::new();
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: None,
        session_key: String::new(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });

    let mut messages = vec![Message::user("call a tool")];
    let result = agent.run_loop(&mut messages).await;

    assert!(
        result.is_ok(),
        "a malformed tool call rejected by the provider must NOT be a fatal turn error: {:?}",
        result
    );
    assert_eq!(
        result.unwrap().response,
        "fixed it",
        "the model should get an addressable next turn to self-correct"
    );
    assert!(
        provider.request_count() >= 2,
        "the model should be re-prompted with addressable feedback (>=2 provider calls), \
         got {}",
        provider.request_count()
    );
    // #2403/#2414: the malformed feedback goes in as its own message, so
    // the prompt already sent is never edited (the Messages API combines
    // consecutive user turns on the wire).
    assert_eq!(
        messages[0].content, "call a tool",
        "the sent prompt is unchanged"
    );
    assert!(
        messages[1].role == crate::domain::message::Role::User
            && messages[1].content.contains("malformed"),
        "the feedback follows the prompt: {:?}",
        messages.iter().map(|m| &m.role).collect::<Vec<_>>()
    );
}

// =======================================================================
// #931: a genuinely-terminal provider error still FAILS the turn
// =======================================================================
//
// Guards against over-broad addressability: only model-malformed `Client`
// 4xx rejections are converted to addressable feedback. A non-addressable
// terminal error (auth, or an exhausted-retries server error) must surface
// as a classified `Err` from run_loop (AC1/AC3), not be silently swallowed.

#[tokio::test]
async fn test_terminal_auth_error_fails_the_turn_with_classified_message() {
    // 401 is never retryable and never addressable — it must fail the turn.
    // Raw text deliberately lacks the word "auth" so the assertion below is
    // satisfied only by the appended Auth-class guidance, not the raw string.
    let responses = vec![Err(DomainError::Provider(
        "provider error (401): invalid credentials".to_string(),
    ))];
    let provider = Arc::new(MockProvider::new_results(responses));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: None,
        session_key: String::new(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });

    let mut messages = vec![Message::user("hi")];
    let result = agent.run_loop(&mut messages).await;

    let err = result.expect_err("a terminal 401 must fail the turn, not be swallowed");
    let lowered = err.to_string().to_ascii_lowercase();
    assert!(
        lowered.contains("authentication") && lowered.contains("re-authenticate"),
        "terminal error should carry classified Auth guidance: {err}"
    );
}

#[tokio::test]
async fn test_terminal_server_error_fails_the_turn_after_retries() {
    // Always 503: retried (provider-attempt budget), then surfaces as a
    // classified terminal Err — never converted to addressable feedback.
    let responses: Vec<Result<LlmResponse, DomainError>> = (0..8)
        .map(|_| {
            Err(DomainError::Provider(
                "provider error (503): service unavailable".to_string(),
            ))
        })
        .collect();
    let provider = Arc::new(MockProvider::new_results(responses));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: None,
        session_key: String::new(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });

    let mut messages = vec![Message::user("hi")];
    let result = agent.run_loop(&mut messages).await;

    let err = result.expect_err("an exhausted server error must fail the turn");
    let lowered = err.to_string().to_ascii_lowercase();
    assert!(
        lowered.contains("server") || lowered.contains("overload"),
        "terminal error should name the server/overload class: {err}"
    );
}

#[test]
fn synthetic_empty_stream_guidance_does_not_claim_http_failure_or_overload() {
    let message = "stream completed without assistant output: synthetic=empty_stream";
    let out = enhance_provider_error(DomainError::Provider(message.into())).to_string();
    assert!(out.contains("synthetic classification"));
    assert!(out.contains("no HTTP failure or overload is established"));
    assert!(!out.contains("provider is overloaded"));
    assert!(!out.contains("HTTP 503"));
}

/// #2435: a model refused for the account says the catalogue now holds it
/// unavailable and how to move on.
#[test]
fn a_model_refused_for_the_account_says_it_will_not_be_offered_again() {
    let out = enhanced(
        r#"HTTP 400 from Codex: {"detail":"The 'mini' model is not supported when using Codex with a ChatGPT account."}"#,
    );
    assert!(
        out.ends_with(
            "Model unavailable: the provider refused this model for the account or auth mode in \
             use. While a refusal is held (providers.model_refusal_ttl_secs, an hour by default; \
             0 holds none) the model is not offered, until the provider serves it again; choose \
             another model (list_models, then set_model)."
        ),
        "{out}"
    );
}

/// #2435 review round 1: the refusal guidance is appended once.
#[test]
fn the_refusal_guidance_is_appended_once() {
    let once = enhanced(
        r#"HTTP 400 from Codex: {"detail":"The 'mini' model is not supported when using Codex with a ChatGPT account."}"#,
    );
    assert_eq!(enhanced(&once), once);
}
