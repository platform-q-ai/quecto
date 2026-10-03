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
