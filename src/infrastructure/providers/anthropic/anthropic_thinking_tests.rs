// Extended thinking tests for Anthropic provider (#175).
// Split from anthropic_tests.rs to stay within the 750-line limit.
//
// Also covers Opus 4.6 API changes:
//   - Adaptive thinking (thinking: {type: "adaptive"}) replaces manual budget_tokens
//   - effort parameter lives in output_config.effort
//   - Pricing: Opus 4.6 is $5/$25 per MTok (not $15/$75)
//   - Cache pricing: write $6.25/MTok, read $0.50/MTok
//   - fine-grained-tool-streaming beta header removed (now GA, no header needed)

use super::*;
use crate::domain::message::Message;
use crate::domain::provider::{ChatRequest, EffortLevel};

#[test]
fn test_build_request_body_with_thinking_adds_thinking_param() {
    let messages = vec![Message::user("Think hard")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 16000,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: Some(crate::domain::provider::ThinkingLevel::Medium),
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 10000);
    // Temperature must be excluded when thinking is enabled
    assert!(
        body.get("temperature").is_none(),
        "temperature must be excluded when thinking is enabled, got: {}",
        body
    );
}

#[test]
fn test_build_request_body_without_thinking_includes_temperature() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert!(body.get("thinking").is_none());
    assert!(
        body.get("temperature").is_some(),
        "temperature should be present when thinking is disabled"
    );
}

#[test]
fn test_build_request_body_thinking_bumps_max_tokens() {
    let messages = vec![Message::user("Think")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-3-5-sonnet-20241022",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: Some(crate::domain::provider::ThinkingLevel::High),
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    // max_tokens must be at least budget_tokens (16384) when thinking is enabled
    assert!(
        body["max_tokens"].as_u64().unwrap() >= 16384,
        "max_tokens should be at least budget_tokens, got: {}",
        body["max_tokens"]
    );
}

#[test]
fn test_parse_sse_response_with_thinking_blocks() {
    let raw = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Let me think about this...\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\"}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Here is my answer\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\"}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":20}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

    let result = AnthropicProvider::parse_sse_response(raw).unwrap();
    // Thinking content should NOT appear in the text content
    assert_eq!(result.content.as_deref(), Some("Here is my answer"));
    // Thinking content is not included in tool_calls
    assert!(result.tool_calls.is_empty());
}

#[test]
fn test_parse_response_with_thinking_blocks() {
    let body = serde_json::json!({
        "content": [
            {"type": "thinking", "thinking": "Let me reason through this..."},
            {"type": "text", "text": "The answer is 42"}
        ],
        "usage": {"input_tokens": 10, "output_tokens": 5},
        "stop_reason": "end_turn"
    });
    let result = AnthropicProvider::parse_response(&body).unwrap();
    assert_eq!(result.content.as_deref(), Some("The answer is 42"));
}

#[test]
fn test_thinking_budget_tokens_levels() {
    use crate::domain::provider::ThinkingLevel;
    assert_eq!(ThinkingLevel::Low.budget_tokens(), Some(1024));
    assert_eq!(ThinkingLevel::Medium.budget_tokens(), Some(10_000));
    assert_eq!(ThinkingLevel::High.budget_tokens(), Some(16_384));
    assert_eq!(ThinkingLevel::Max.budget_tokens(), Some(32_768));
    assert_eq!(ThinkingLevel::Adaptive.budget_tokens(), None);
}

#[test]
fn test_thinking_budget_tokens_adaptive_returns_none() {
    use crate::domain::provider::ThinkingLevel;
    // budget_tokens() must return None for Adaptive — not panic.
    assert_eq!(ThinkingLevel::Adaptive.budget_tokens(), None);
}

// ---------------------------------------------------------------------------
// Opus 4.6: adaptive thinking
// ---------------------------------------------------------------------------

/// Opus 4.6 with ThinkingLevel::Adaptive should emit thinking:{type:"adaptive"}
/// and must NOT include budget_tokens or temperature.
#[test]
fn test_opus_4_6_adaptive_thinking_emits_correct_json() {
    let messages = vec![Message::user("Think hard")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 16_000,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: Some(crate::domain::provider::ThinkingLevel::Adaptive),
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(
        body["thinking"]["type"], "adaptive",
        "Opus 4.6 should use adaptive thinking, got: {}",
        body["thinking"]
    );
    assert!(
        body["thinking"].get("budget_tokens").is_none(),
        "adaptive thinking must not include budget_tokens"
    );
    assert!(
        body.get("temperature").is_none(),
        "temperature must be excluded when thinking is enabled"
    );
}

/// Adaptive thinking should NOT include budget_tokens even on sonnet 4.6.
#[test]
fn test_sonnet_4_6_adaptive_thinking_emits_correct_json() {
    let messages = vec![Message::user("Reason please")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-6",
        max_tokens: 16_000,
        temperature: 0.5,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: Some(crate::domain::provider::ThinkingLevel::Adaptive),
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert!(body["thinking"].get("budget_tokens").is_none());
}

/// Older models (Opus 4.5, Sonnet 4.5) still use manual thinking with budget_tokens.
#[test]
fn test_older_model_manual_thinking_still_uses_budget_tokens() {
    let messages = vec![Message::user("Think")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-5-20251101",
        max_tokens: 16_000,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: Some(crate::domain::provider::ThinkingLevel::High),
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert!(
        body["thinking"].get("budget_tokens").is_some(),
        "older models must still use budget_tokens"
    );
    assert_eq!(body["thinking"]["budget_tokens"], 16_384);
}

// ---------------------------------------------------------------------------
// Opus 4.6: effort parameter in output_config
// ---------------------------------------------------------------------------

/// effort is emitted as output_config.effort (not top-level or inside thinking).
#[test]
fn test_effort_emitted_in_output_config() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 4_096,
        temperature: 1.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: Some(EffortLevel::Medium),
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(
        body["output_config"]["effort"], "medium",
        "effort should be at output_config.effort, got body: {}",
        body
    );
    // Must NOT appear at the top level or inside thinking
    assert!(
        body.get("effort").is_none(),
        "effort must not be a top-level field"
    );
}

#[test]
fn test_effort_low_emitted_correctly() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 4_096,
        temperature: 1.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: Some(EffortLevel::Low),
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(body["output_config"]["effort"], "low");
}

#[test]
fn test_effort_high_emitted_correctly() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 4_096,
        temperature: 1.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: Some(EffortLevel::High),
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(body["output_config"]["effort"], "high");
}

#[test]
fn test_effort_max_emitted_correctly() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 4_096,
        temperature: 1.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: Some(EffortLevel::Max),
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(body["output_config"]["effort"], "max");
}

/// When effort is None, output_config should not be emitted.
#[test]
fn test_no_effort_omits_output_config() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 4_096,
        temperature: 1.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert!(
        body.get("output_config").is_none(),
        "output_config should be absent when effort is None, got: {}",
        body
    );
}

/// Adaptive thinking + effort together produce the correct combined payload.
#[test]
fn test_adaptive_thinking_with_effort_combined() {
    let messages = vec![Message::user("Complex task")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 16_000,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: Some(crate::domain::provider::ThinkingLevel::Adaptive),
        cancel_flag: None,
        effort: Some(EffortLevel::High),
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert!(body["thinking"].get("budget_tokens").is_none());
    assert_eq!(body["output_config"]["effort"], "high");
    assert!(body.get("temperature").is_none());
}

// ---------------------------------------------------------------------------
// Pricing: Opus 4.6 corrected rates ($5/$25, cache $6.25/$0.50 per MTok)
// ---------------------------------------------------------------------------

#[test]
fn test_opus_4_6_pricing_is_five_dollars_input() {
    use crate::domain::message::{UsageInfo, model_pricing};
    let pricing = model_pricing("claude-opus-4-6").expect("claude-opus-4-6 should have pricing");
    // $5.00 / MTok input
    assert_eq!(
        pricing.input_micro_usd_per_million, 5_000_000,
        "Opus 4.6 input should be $5/MTok (5_000_000 micro-USD), got {}",
        pricing.input_micro_usd_per_million
    );
    // $25.00 / MTok output
    assert_eq!(
        pricing.output_micro_usd_per_million, 25_000_000,
        "Opus 4.6 output should be $25/MTok"
    );
    // Cache write: $6.25 / MTok (1.25x base)
    assert_eq!(
        pricing.cache_write_micro_usd_per_million, 6_250_000,
        "Opus 4.6 cache write should be $6.25/MTok"
    );
    // Cache read: $0.50 / MTok (0.1x base)
    assert_eq!(
        pricing.cache_read_micro_usd_per_million, 500_000,
        "Opus 4.6 cache read should be $0.50/MTok"
    );

    // Spot-check cost calculation: 1M input tokens = $5.00
    let usage = UsageInfo {
        prompt_tokens: 1_000_000,
        completion_tokens: 0,
        cache_read_tokens: None,
        cache_write_tokens: None,
        cost: None,
    };
    let cost = pricing.cost_for(&usage);
    assert_eq!(cost.input_cost_micro_usd, 5_000_000);
    assert!((cost.input_cost_usd() - 5.0).abs() < 1e-6);
}

#[test]
fn test_opus_4_6_cache_read_pricing() {
    use crate::domain::message::{UsageInfo, model_pricing};
    let pricing = model_pricing("claude-opus-4-6").unwrap();
    // 1M cache-read tokens = $0.50
    let usage = UsageInfo {
        prompt_tokens: 0,
        completion_tokens: 0,
        cache_read_tokens: Some(1_000_000),
        cache_write_tokens: None,
        cost: None,
    };
    let cost = pricing.cost_for(&usage);
    assert_eq!(cost.cache_read_cost_micro_usd, 500_000);
    assert!((cost.cache_read_cost_usd() - 0.5).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// Pricing: Haiku 4.5 ($1/$5 per MTok, cache $1.25/$0.10)
// ---------------------------------------------------------------------------

#[test]
fn test_haiku_4_5_pricing_present_and_correct() {
    use crate::domain::message::{UsageInfo, model_pricing};
    let pricing = model_pricing("claude-haiku-4-5").expect("claude-haiku-4-5 should have pricing");
    assert_eq!(
        pricing.input_micro_usd_per_million, 1_000_000,
        "$1/MTok input"
    );
    assert_eq!(
        pricing.output_micro_usd_per_million, 5_000_000,
        "$5/MTok output"
    );
    assert_eq!(
        pricing.cache_write_micro_usd_per_million, 1_250_000,
        "$1.25/MTok cache write"
    );
    assert_eq!(
        pricing.cache_read_micro_usd_per_million, 100_000,
        "$0.10/MTok cache read"
    );

    let usage = UsageInfo {
        prompt_tokens: 1_000_000,
        completion_tokens: 1_000_000,
        cache_read_tokens: None,
        cache_write_tokens: None,
        cost: None,
    };
    let cost = pricing.cost_for(&usage);
    assert!((cost.input_cost_usd() - 1.0).abs() < 1e-6);
    assert!((cost.output_cost_usd() - 5.0).abs() < 1e-6);
}

#[test]
fn test_haiku_4_5_dated_variant_matches() {
    use crate::domain::message::model_pricing;
    assert!(model_pricing("claude-haiku-4-5-20251001").is_some());
}

// ---------------------------------------------------------------------------
// Beta header: fine-grained-tool-streaming removed (now GA)
// ---------------------------------------------------------------------------

/// API-key auth must NOT send the fine-grained-tool-streaming beta header
/// since it graduated to GA and should be removed per migration guide.
#[tokio::test]
async fn test_api_key_auth_does_not_send_fine_grained_streaming_beta_header() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response_body = serde_json::json!({
        "id": "msg_ga",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 2}
    });
    // This mock only matches if the beta header is ABSENT — if the header is
    // present wiremock won't match and the test will get a 404, failing it.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .expect(1)
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new("sk-ant-test".to_string(), Some(server.uri()));
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 1024,
        temperature: 1.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    // The call must succeed; if the beta header were sent we'd need a matcher
    // for it but the mock above has no header matcher, so it always matches.
    // The real assertion is that `apply_auth_headers` no longer sets
    // "fine-grained-tool-streaming-2025-05-14" at all — checked by
    // inspecting the captured request below.
    let result = provider.chat(req).await;
    assert!(result.is_ok(), "chat should succeed: {:?}", result);

    // Inspect the single captured request for the absence of the old header.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    let beta = req
        .headers
        .get("anthropic-beta")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        !beta.contains("fine-grained-tool-streaming"),
        "fine-grained-tool-streaming beta header should have been removed (now GA), got: {:?}",
        beta
    );
}
