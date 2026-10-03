//! #2436: an ended provider request reaches the agent's own socket as one
//! `request_completed` event, with the token counts its provider reported
//! and none it did not.
use super::*;
use crate::domain::inference::request_completion::{
    RequestCompleted, RequestOutcome, RequestSpend,
};
use serde_json::json;

fn ended(spend: Option<RequestSpend>, outcome: RequestOutcome) -> AgentProgressEvent {
    AgentProgressEvent::RequestCompleted(RequestCompleted {
        model: "gpt-5.5".into(),
        provider: "openai-oauth".into(),
        spend,
        duration_ms: 1234,
        outcome,
        request_index: 7,
        attempt: 2,
    })
}

async fn written(events: Vec<AgentProgressEvent>, settled: bool) -> Vec<serde_json::Value> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut sink = EventSink::writer(&mut buf);
        match settled {
            true => {
                let (tx, mut rx) = tokio::sync::mpsc::channel(8);
                for event in events {
                    tx.try_send(event).unwrap();
                }
                forward_settled_requests(&mut rx, &mut sink).await;
            }
            false => {
                for event in events {
                    forward_event(event, &mut sink).await;
                }
            }
        }
    }
    String::from_utf8(buf)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn an_ended_request_is_one_request_completed_event() {
    let spend = RequestSpend {
        input_tokens: 1200,
        cached_tokens: Some(800),
        cache_write_tokens: None,
        output_tokens: 45,
    };
    let lines = written(vec![ended(Some(spend), RequestOutcome::Ok)], false).await;
    assert_eq!(
        lines,
        [json!({
            "type": "request_completed",
            "model": "gpt-5.5",
            "provider": "openai-oauth",
            "inputTokens": 1200,
            "cachedTokens": 800,
            "outputTokens": 45,
            "durationMs": 1234,
            "outcome": "ok",
            "requestIndex": 7,
            "attempt": 2
        })]
    );
}

#[tokio::test]
async fn token_counts_a_provider_did_not_report_are_omitted() {
    let lines = written(vec![ended(None, RequestOutcome::Error)], false).await;
    assert_eq!(
        lines,
        [json!({
            "type": "request_completed",
            "model": "gpt-5.5",
            "provider": "openai-oauth",
            "durationMs": 1234,
            "outcome": "error",
            "requestIndex": 7,
            "attempt": 2
        })]
    );
    let uncached = RequestSpend {
        input_tokens: 3,
        cached_tokens: None,
        cache_write_tokens: None,
        output_tokens: 0,
    };
    let lines = written(vec![ended(Some(uncached), RequestOutcome::Ok)], false).await;
    assert!(lines[0].get("cachedTokens").is_none(), "{}", lines[0]);
    assert_eq!(lines[0]["outputTokens"], 0, "a reported zero stays");
}

/// A cancelled turn's drain has stopped: the requests that ended as it was
/// dropped are still announced, and nothing else it queued.
#[tokio::test]
async fn a_cancelled_turn_still_announces_its_ended_requests() {
    let lines = written(
        vec![
            AgentProgressEvent::Token("late".into()),
            ended(None, RequestOutcome::Cancelled),
            AgentProgressEvent::Done,
        ],
        true,
    )
    .await;
    let types: Vec<_> = lines.iter().map(|line| line["type"].clone()).collect();
    assert_eq!(types, [json!("request_completed")]);
    assert_eq!(lines[0]["outcome"], "cancelled");
}

/// #2436 review M2: cache writes ride as their own count.
#[tokio::test]
async fn cache_writes_ride_as_their_own_count() {
    let spend = RequestSpend {
        input_tokens: 100,
        cached_tokens: Some(1000),
        cache_write_tokens: Some(5000),
        output_tokens: 8,
    };
    let lines = written(vec![ended(Some(spend), RequestOutcome::Ok)], false).await;
    assert_eq!(lines[0]["inputTokens"], 100);
    assert_eq!(lines[0]["cachedTokens"], 1000);
    assert_eq!(lines[0]["cacheWriteTokens"], 5000);
}

/// A provider that never answers, and says when it was asked.
#[derive(Debug)]
struct Hanging(std::sync::Arc<tokio::sync::Notify>);

impl crate::application::providers::ports::LlmProvider for Hanging {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "hanging"
    }
    fn chat<'a>(
        &'a self,
        _request: crate::application::providers::ports::ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::domain::message::LlmResponse,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.0.notify_one();
        Box::pin(std::future::pending())
    }
}

/// #2436 review L5: `abort` cancels the prompt run while its request is in
/// flight; the cancelled request still reaches the socket.
#[tokio::test]
async fn an_aborted_turn_puts_a_cancelled_request_on_the_socket() {
    use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use crate::interface::cli::uds_cancel::{PromptOutcome, PromptRun, run_agent_message};
    let asked = std::sync::Arc::new(tokio::sync::Notify::new());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: std::sync::Arc::new(Hanging(asked.clone())),
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: "cli:test".into(),
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
    let mut messages = Vec::new();
    let mut session = crate::interface::cli::uds_session::AgentSession::new("stub".into());
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        asked.notified().await;
        let _ = cancel_tx.send(());
    });
    let mut notification_rx = None;
    let mut buf: Vec<u8> = Vec::new();
    let outcome = {
        let mut sink = EventSink::writer(&mut buf);
        run_agent_message(PromptRun {
            execution_state: None,
            agent: &mut agent,
            messages: &mut messages,
            active_session: None,
            session: &mut session,
            sink: &mut sink,
            message: crate::domain::message::Message::user("hello"),
            system_prompt: "",
            cancel_rx,
            notification_rx: &mut notification_rx,
            subagent_registry: &None,
            turn_save: None,
        })
        .await
    };
    assert!(matches!(outcome, PromptOutcome::Cancelled));
    let completed: Vec<serde_json::Value> = String::from_utf8(buf)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["type"] == "request_completed")
        .collect();
    assert_eq!(completed.len(), 1, "{completed:?}");
    assert_eq!(completed[0]["outcome"], "cancelled");
    assert_eq!(completed[0]["requestIndex"], 1);
    assert_eq!(agent.request_tally().counters().requests, 1);
}
