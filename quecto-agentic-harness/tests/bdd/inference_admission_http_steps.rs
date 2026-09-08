//! Real loopback HTTP contracts for each admitted provider entrypoint.
use super::inference_admission_provider_steps::feedback;
use super::*;
use quecto::domain::inference_admission::{Feedback, ThrottleFeedback};

#[derive(Debug, Default)]
pub struct HttpAdmissionState {
    leaf: Option<feedback::Leaf>,
    surface: String,
    case: String,
    output: Option<feedback::Transcript>,
    before: Option<feedback::Snapshot>,
    after: Option<feedback::Snapshot>,
    starts: usize,
    completed_early: bool,
}

#[given(
    regex = r"^an admitted (OpenAI|Anthropic|Codex-key|Codex-oauth) (chat|assembled|incremental) HTTP call$"
)]
fn configured(w: &mut QuectoWorld, leaf: String, surface: String) {
    w.http_admission.leaf = Some(match leaf.as_str() {
        "OpenAI" => feedback::Leaf::OpenAi,
        "Anthropic" => feedback::Leaf::Anthropic,
        "Codex-key" => feedback::Leaf::Responses,
        "Codex-oauth" => feedback::Leaf::OAuth,
        _ => unreachable!(),
    });
    w.http_admission.surface = surface;
}

fn success_body(leaf: feedback::Leaf, surface: &str) -> String {
    use feedback::Leaf::*;
    if surface == "chat" {
        match leaf {
            OpenAi => return r#"{"choices":[{"message":{"content":"visible once"},"finish_reason":"stop"}]}"#.into(),
            Anthropic => return r#"{"content":[{"type":"text","text":"visible once"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#.into(),
            _ => {}
        }
    }
    match leaf {
        OpenAi => "data: {\"choices\":[{\"delta\":{\"content\":\"visible once\"}}]}\n\ndata: [DONE]\n\n",
        Anthropic => "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"visible once\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        Responses | OAuth => "data: {\"type\":\"response.output_text.delta\",\"delta\":\"visible once\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
    }.into()
}

#[when(
    regex = r"^the loopback server returns (success|opaque-throttle|terminal-billing|header-throttle)$"
)]
fn execute(w: &mut QuectoWorld, case: String) {
    let s = &mut w.http_admission;
    s.case = case.clone();
    let leaf = s.leaf.unwrap();
    let surface = s.surface.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let gate = Arc::new(feedback::Gate::default());
        let body = match case.as_str() {
            "success" => success_body(leaf, &surface),
            "terminal-billing" => r#"{"error":{"type":"rate_limit_error","code":"insufficient_quota","message":"billing is terminal"}}"#.into(),
            _ => "opaque busy".into(),
        };
        let stalled = case == "header-throttle";
        let mut server = feedback::Server::start(feedback::Reply {
            status: if case == "success" { 200 } else { 429 },
            headers: if stalled { "Retry-After: 30\r\n" } else { "Content-Type: text/event-stream\r\n" }.into(),
            body,
            stalled,
        }).await;
        let provider = leaf.provider(&server.url, gate.clone());
        let mut task = feedback::Task(tokio::spawn(async move {
            if surface == "incremental" { return feedback::stream(provider.as_ref()).await; }
            let result = if surface == "chat" {
                provider.chat(feedback::request()).await
            } else {
                provider.chat_stream(feedback::request()).await
            };
            match result {
                Ok(response) => feedback::Transcript { text: vec![response.content.unwrap_or_default()], done: 1, errors: vec![] },
                Err(error) => feedback::Transcript { errors: vec![error.to_string()], ..Default::default() },
            }
        }));
        let connection = server.connection().await;
        if stalled {
            gate.observe_receipt().await;
            s.before = Some(gate.snapshot());
            s.completed_early = task.0.is_finished();
        }
        connection.release();
        s.output = Some(feedback::bounded(&mut task.0).await.unwrap());
        s.after = Some(gate.snapshot());
        s.starts = server.starts();
    });
}

#[then("HTTP admission records exactly one owned outcome without replay")]
fn outcome(w: &mut QuectoWorld) {
    let s = &w.http_admission;
    let after = s.after.as_ref().unwrap();
    let out = s.output.as_ref().unwrap();
    assert_eq!(s.starts, 1, "one physical POST");
    assert_eq!(after.queued, 1);
    assert_eq!(after.grants, 1);
    assert_eq!(after.abandoned, 0);
    let success = s.case == "success";
    assert_eq!(
        after.finishes,
        vec![if success {
            Feedback::Success
        } else {
            Feedback::Failure
        }]
    );
    if success {
        assert_eq!(out.text, ["visible once"]);
        assert_eq!(out.done, 1);
        assert!(out.errors.is_empty());
    } else {
        assert!(out.text.is_empty());
        assert_eq!(out.done, 0);
        assert_eq!(out.errors.len(), 1);
        assert!(
            out.errors[0].contains("429"),
            "original HTTP status: {:?}",
            out.errors
        );
        assert!(out.errors[0].contains(if s.case == "terminal-billing" {
            "billing is terminal"
        } else {
            "opaque busy"
        }));
    }
    match s.case.as_str() {
        "success" | "terminal-billing" => assert!(after.receipts.is_empty()),
        "opaque-throttle" => {
            assert_eq!(after.receipts.len(), 1);
            assert!(matches!(after.receipts[0], ThrottleFeedback::NoHint { .. }));
        }
        "header-throttle" => {
            let before = s.before.as_ref().unwrap();
            assert_eq!(before.grants, 1);
            assert!(
                before.finishes.is_empty(),
                "receipt precedes body completion"
            );
            assert!(!s.completed_early);
            assert_eq!(before.receipts.len(), 1);
            assert!(matches!(before.receipts[0], ThrottleFeedback::Until(_)));
            assert_eq!(
                after.receipts, before.receipts,
                "header feedback is not duplicated at EOF"
            );
        }
        _ => unreachable!(),
    }
}
