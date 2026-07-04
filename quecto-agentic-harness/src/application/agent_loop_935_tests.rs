//! #935: per-model `max_tokens` clamp.
//!
//! The effective per-request output cap must be `min(configured_max_tokens,
//! model_registry.max_tokens)` when the model has a known registry cap, for
//! both the OpenAI-compatible (`max_completion_tokens`) and Anthropic
//! (`max_tokens`) paths. This is the application-layer seam: the
//! `AgentLoopImpl` clamps `ChatRequest.max_tokens` before handing it to the
//! provider. These tests assert the request the provider actually receives.
//!
//! They FAIL before the fix because `build_chat_request` sends
//! `self.max_tokens` verbatim (e.g. 200_000), ignoring the model cap.

use super::*;
use crate::domain::message::Message;

/// Build an agent with an explicit configured `max_tokens` and an optional
/// per-model registry cap, plus a single text response so `run_loop` does one
/// provider call we can inspect.
fn agent_with_caps(
    configured_max_tokens: u32,
    model_max_tokens: Option<u32>,
) -> (AgentLoopImpl, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider::new(vec![text_response("done")]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::new()),
        model: "fireworks/qwen3p7-plus".to_string(),
        max_tokens: configured_max_tokens,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    })
    .with_model_max_tokens(model_max_tokens);
    (agent, provider)
}

/// Acceptance criterion 1: configured 200_000 + model cap 65_536 ⇒ the request
/// carries 65_536, never the larger configured value.
#[tokio::test]
async fn request_output_cap_is_clamped_to_model_cap() {
    let (agent, provider) = agent_with_caps(200_000, Some(65_536));
    let mut messages = vec![Message::user("hi")];
    agent.run_loop(&mut messages).await.unwrap();

    let seen = provider.seen_max_tokens();
    assert_eq!(
        seen,
        vec![65_536],
        "request output cap must be min(200000, 65536) = 65536, got {seen:?}"
    );
}

/// Acceptance criterion 2: a model without a registry cap keeps the configured
/// value (no regression).
#[tokio::test]
async fn request_output_cap_unchanged_without_model_cap() {
    let (agent, provider) = agent_with_caps(8_192, None);
    let mut messages = vec![Message::user("hi")];
    agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(
        provider.seen_max_tokens(),
        vec![8_192],
        "with no model cap the configured max_tokens must be sent verbatim"
    );
}

/// When the configured value is already below the model cap it is left intact
/// (clamp is a floor by min, never a raise).
#[tokio::test]
async fn request_output_cap_not_raised_to_model_cap() {
    let (agent, provider) = agent_with_caps(4_096, Some(65_536));
    let mut messages = vec![Message::user("hi")];
    agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(
        provider.seen_max_tokens(),
        vec![4_096],
        "clamp must never raise the configured value toward the model cap"
    );
}

/// Acceptance criterion 3 (application half): once the per-model cap is updated
/// via `set_model`, the next turn re-clamps. This proves the switch path the
/// interface uses; the registry lookup that feeds it is proven by
/// `model_registry_tests::max_tokens_for_*`.
#[tokio::test]
async fn set_model_max_tokens_re_clamps_for_subsequent_turns() {
    let provider = Arc::new(MockProvider::new(vec![
        text_response("turn-1"),
        text_response("turn-2"),
    ]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::new()),
        model: "fireworks/big-model".to_string(),
        max_tokens: 200_000,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    });
    // First turn: no per-model cap → configured value.
    let mut m1 = vec![Message::user("a")];
    agent.run_loop(&mut m1).await.unwrap();

    // Switch to a lower-cap model and re-clamp.
    agent.set_model("fireworks/qwen3p7-plus".to_string(), Some(65_536));
    let mut m2 = vec![Message::user("b")];
    agent.run_loop(&mut m2).await.unwrap();

    assert_eq!(
        provider.seen_max_tokens(),
        vec![200_000, 65_536],
        "after a switch to a lower-cap model the next turn must re-clamp to 65536"
    );
}

/// `effective_max_tokens` directly reflects the clamp policy.
#[test]
fn effective_max_tokens_is_min_of_configured_and_model_cap() {
    let (clamped, _) = agent_with_caps(200_000, Some(65_536));
    assert_eq!(clamped.effective_max_tokens(), 65_536);

    let (uncapped, _) = agent_with_caps(8_192, None);
    assert_eq!(uncapped.effective_max_tokens(), 8_192);
}
