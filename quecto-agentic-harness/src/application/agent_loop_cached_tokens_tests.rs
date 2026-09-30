//! #2348: `llm_turn_end` records the share of the input the provider
//! served from its prompt cache, for every provider that reports it, so
//! the improvement loop can weigh a prune's token saving against the
//! cache miss its prefix rewrite costs.

use crate::application::agent_loop::agent_loop_pruning::ctx_mgmt_tests::CapturingAuditSink;
use crate::application::agent_loop::tests::{MockProvider, MockRegistry, test_config};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::audit::ports::AuditSink;
use crate::domain::audit::AuditEvent;
use crate::domain::message::{LlmResponse, Message, UsageInfo};
use crate::infrastructure::providers::usage::{parse_codex_usage, parse_openai_usage};
use std::sync::Arc;

/// The `llm_turn_end` records of one turn answered with `usage`.
async fn turn_end_records(usage: Option<UsageInfo>) -> Vec<AuditEvent> {
    let sink = Arc::new(CapturingAuditSink::default());
    let provider = Arc::new(MockProvider::new(vec![LlmResponse {
        content: Some("done".to_string()),
        tool_calls: vec![],
        usage,
        stop_reason: None,
        thinking_blocks: vec![],
    }]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        audit_log: Some(sink.clone() as Arc<dyn AuditSink>),
        ..test_config(provider, Box::new(MockRegistry::new()))
    });
    let mut messages = vec![Message::user("hi")];
    agent.run_loop(&mut messages).await.unwrap();
    let events = sink.events.lock().unwrap();
    events
        .iter()
        .filter(|event| matches!(event, AuditEvent::LlmTurnEnd { .. }))
        .cloned()
        .collect()
}

fn cached_and_input(records: &[AuditEvent]) -> Vec<(Option<usize>, usize)> {
    records
        .iter()
        .filter_map(|event| match event {
            AuditEvent::LlmTurnEnd {
                cached_input_tokens,
                input_tokens,
                ..
            } => Some((*cached_input_tokens, *input_tokens)),
            _ => None,
        })
        .collect()
}

fn usage_object(json: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str(json).unwrap()
}

#[tokio::test]
async fn openai_chat_cached_tokens_are_recorded_as_a_share_of_the_input() {
    let usage = parse_openai_usage(&usage_object(
        r#"{"prompt_tokens":9000,"completion_tokens":20,
            "prompt_tokens_details":{"cached_tokens":6000}}"#,
    ));
    let records = turn_end_records(Some(usage)).await;
    assert_eq!(cached_and_input(&records), vec![(Some(6_000), 9_000)]);
}

#[tokio::test]
async fn codex_responses_cached_tokens_are_recorded() {
    let usage = parse_codex_usage(&usage_object(
        r#"{"input_tokens":12000,"output_tokens":40,
            "input_tokens_details":{"cached_tokens":11000}}"#,
    ));
    let records = turn_end_records(Some(usage)).await;
    assert_eq!(cached_and_input(&records), vec![(Some(11_000), 12_000)]);
}

#[tokio::test]
async fn anthropic_cache_reads_are_recorded() {
    // As the Anthropic adapter normalizes `cache_read_input_tokens`: the
    // input is the uncached delta plus the cache read and write.
    let usage = UsageInfo {
        prompt_tokens: 500,
        completion_tokens: 30,
        cache_read_tokens: Some(7_000),
        cache_write_tokens: Some(1_500),
        context_tokens: Some(9_000),
        cost: None,
    };
    let records = turn_end_records(Some(usage)).await;
    assert_eq!(cached_and_input(&records), vec![(Some(7_000), 9_000)]);
}

#[tokio::test]
async fn no_cached_share_is_recorded_when_the_provider_reports_none() {
    let usage = parse_openai_usage(&usage_object(
        r#"{"prompt_tokens":9000,"completion_tokens":20}"#,
    ));
    let records = turn_end_records(Some(usage)).await;
    assert_eq!(cached_and_input(&records), vec![(None, 9_000)]);
    let json = serde_json::to_string(&records[0]).unwrap();
    assert!(!json.contains("cached_input_tokens"), "{json}");

    let estimated = turn_end_records(None).await;
    assert_eq!(cached_and_input(&estimated)[0].0, None);
}

#[test]
fn a_record_written_before_the_field_reads_as_none() {
    let old = r#"{"event":"llm_turn_end","input_tokens":10,"output_tokens":2,
        "stop_reason":"end_turn","duration_ms":5}"#;
    let event: AuditEvent = serde_json::from_str(old).unwrap();
    assert_eq!(cached_and_input(&[event]), vec![(None, 10)]);
}
