//! #2348: `llm_turn_end` records the share of the input the provider
//! served from its prompt cache, for every provider that reports it, so
//! the improvement loop can weigh a prune's token saving against the
//! cache miss its prefix rewrite costs.

use crate::application::agent_loop::agent_loop_pruning::ctx_mgmt_tests::CapturingAuditSink;
use crate::application::agent_loop::tests::{MockProvider, MockRegistry, test_config};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::audit::ports::AuditSink;
use crate::domain::audit::AuditEvent;
use crate::domain::conversation::value_objects::message::{LlmResponse, Message, UsageInfo};
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

/// A turn's usage as a provider adapter normalizes it: `prompt_tokens`
/// is the uncached input, `context_tokens` the whole prompt.
fn usage(
    uncached: u32,
    cache_read: Option<u32>,
    cache_write: Option<u32>,
    context: u32,
) -> UsageInfo {
    UsageInfo {
        prompt_tokens: uncached,
        completion_tokens: 20,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        context_tokens: Some(context),
        cost: None,
    }
}

fn cache_writes(records: &[AuditEvent]) -> Vec<Option<usize>> {
    records
        .iter()
        .filter_map(|event| match event {
            AuditEvent::LlmTurnEnd {
                cache_write_tokens, ..
            } => Some(*cache_write_tokens),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn openai_chat_cached_tokens_are_recorded_as_a_share_of_the_input() {
    // As `providers::usage::parse_openai_usage` normalizes
    // `prompt_tokens_details.cached_tokens` (pinned by its `usage_tests`).
    let records = turn_end_records(Some(usage(3_000, Some(6_000), None, 9_000))).await;
    assert_eq!(cached_and_input(&records), vec![(Some(6_000), 9_000)]);
    assert_eq!(cache_writes(&records), vec![None]);
}

#[tokio::test]
async fn codex_responses_cached_tokens_are_recorded() {
    // As `providers::usage::parse_codex_usage` normalizes
    // `input_tokens_details.cached_tokens` (pinned by its `usage_tests`).
    let records = turn_end_records(Some(usage(1_000, Some(11_000), None, 12_000))).await;
    assert_eq!(cached_and_input(&records), vec![(Some(11_000), 12_000)]);
}

#[tokio::test]
async fn anthropic_cache_reads_and_writes_are_recorded() {
    // As the Anthropic adapter normalizes `cache_read_input_tokens` and
    // `cache_creation_input_tokens`: the input is the uncached delta plus
    // the cache read and write. The parsers are pinned by
    // `anthropic_tests::test_parse_sse_extracts_cache_usage` (streamed) and
    // `anthropic_tests::test_parse_response_extracts_cache_usage` (not).
    let records = turn_end_records(Some(usage(500, Some(7_000), Some(1_500), 9_000))).await;
    assert_eq!(cached_and_input(&records), vec![(Some(7_000), 9_000)]);
    assert_eq!(cache_writes(&records), vec![Some(1_500)]);
}

#[tokio::test]
async fn no_cached_share_is_recorded_when_the_provider_reports_none() {
    let records = turn_end_records(Some(usage(9_000, None, None, 9_000))).await;
    assert_eq!(cached_and_input(&records), vec![(None, 9_000)]);
    let json = serde_json::to_string(&records[0]).unwrap();
    assert!(!json.contains("cached_input_tokens"), "{json}");
    assert!(!json.contains("cache_write_tokens"), "{json}");

    let estimated = turn_end_records(None).await;
    assert_eq!(cached_and_input(&estimated)[0].0, None);
    assert_eq!(cache_writes(&estimated), vec![None]);
}

#[test]
fn a_record_written_before_the_field_reads_as_none() {
    let old = r#"{"event":"llm_turn_end","input_tokens":10,"output_tokens":2,
        "stop_reason":"end_turn","duration_ms":5}"#;
    let event: AuditEvent = serde_json::from_str(old).unwrap();
    assert_eq!(cached_and_input(&[event]), vec![(None, 10)]);
}
