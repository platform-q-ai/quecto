//! #2459 and #2531 through the agent turn: a terminal failure's guidance
//! says what the request did (its attempts, whether output streamed), and
//! an Envoy 507 for an exhausted upstream retry buffer is retried before
//! any output like any server failure.
use crate::application::agent_turn::agent_loop::tests::{
    MockRegistry, MockStreamingProvider, test_config, text_response,
};
use crate::application::agent_turn::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::inference::value_objects::provider::StreamEvent;
use std::sync::Arc;

const CUT_SHORT: &str =
    "OpenAI SSE stream ended without completion: connection closed before [DONE]";

fn streaming(responses: Vec<Vec<StreamEvent>>) -> (AgentLoopImpl, Arc<MockStreamingProvider>) {
    let provider = Arc::new(MockStreamingProvider::new(responses));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        streaming: true,
        ..test_config(provider.clone(), Box::new(MockRegistry::default()))
    });
    (agent, provider)
}

#[tokio::test]
async fn a_stream_cut_off_after_output_says_it_was_not_retried() {
    let (mut agent, provider) = streaming(vec![vec![
        StreamEvent::TextDelta("half an answ".into()),
        StreamEvent::Error(CUT_SHORT.into()),
    ]]);
    let mut messages = vec![Message::user("hello")];
    let err = agent
        .run_loop(&mut messages)
        .await
        .expect_err("a reply cut off after output ends the turn");
    let text = err.to_string();
    assert_eq!(provider.request_count(), 1, "never replayed");
    assert!(text.contains("It was not retried"), "{text}");
    assert!(
        text.contains("continuing or re-prompting is safe"),
        "{text}"
    );
    assert!(!text.contains("It was retried"), "{text}");
    assert!(!text.contains("could not reach"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn a_failure_before_output_after_retries_states_the_attempt_count() {
    let (mut agent, provider) = streaming(vec![
        vec![StreamEvent::Error(CUT_SHORT.into())],
        vec![StreamEvent::Error(CUT_SHORT.into())],
        vec![StreamEvent::Error(CUT_SHORT.into())],
    ]);
    let mut messages = vec![Message::user("hello")];
    let err = agent.run_loop(&mut messages).await.expect_err("exhausted");
    let text = err.to_string();
    assert_eq!(provider.request_count(), 3);
    assert!(text.contains("It was retried (3 attempts)"), "{text}");
    assert!(
        !text.contains("could not reach"),
        "no attempt record says the provider was unreachable: {text}"
    );
}

#[tokio::test(start_paused = true)]
async fn an_envoy_507_retry_buffer_limit_before_output_is_retried() {
    let (mut agent, provider) = streaming(vec![
        vec![StreamEvent::Error(
            "HTTP 507 from Codex: exceeded request buffer limit while retrying upstream".into(),
        )],
        vec![StreamEvent::Done(text_response("recovered"))],
    ]);
    let mut messages = vec![Message::user("hello")];
    let result = agent
        .run_loop(&mut messages)
        .await
        .expect("a 507 retry-buffer limit is transient");
    assert_eq!(result.response, "recovered");
    assert_eq!(provider.request_count(), 2);
}

/// A request admission gate that admits the first send and refuses every
/// reattempt, or refuses at once.
#[derive(Debug)]
struct Gate {
    refuse_first: bool,
}

impl crate::application::providers::ports::RequestAdmission for Gate {
    fn check(
        &self,
        attempt: crate::domain::inference::value_objects::provider::RequestAttempt,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::domain::shared::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        use crate::domain::inference::value_objects::provider::RequestAttempt;
        let refused = match attempt {
            RequestAttempt::First => self.refuse_first,
            RequestAttempt::Reattempt => true,
        };
        Box::pin(async move {
            match refused {
                true => Err(crate::domain::shared::error::DomainError::Provider(
                    "run admission refused: the connection to the run board closed".into(),
                )),
                false => Ok(()),
            }
        })
    }
}

/// #2600 review: the request admission gate's refusal, on the first send
/// or on a reattempt, is returned as it is: no provider guidance.
#[tokio::test(start_paused = true)]
async fn an_admission_refusal_gets_no_provider_guidance() {
    for refuse_first in [true, false] {
        let (agent, provider) = streaming(vec![
            vec![StreamEvent::Error(CUT_SHORT.into())],
            vec![StreamEvent::Done(text_response("unused"))],
        ]);
        let mut agent = agent.with_request_admission(Some(Arc::new(Gate { refuse_first })));
        let mut messages = vec![Message::user("hello")];
        let err = agent.run_loop(&mut messages).await.expect_err("refused");
        assert_eq!(
            err.to_string(),
            "provider error: run admission refused: the connection to the run board closed",
            "refuse_first={refuse_first}"
        );
        let sent = if refuse_first { 0 } else { 1 };
        assert_eq!(provider.request_count(), sent);
    }
}

fn record(
    wire_status: Option<u16>,
) -> crate::domain::inference::value_objects::attempt_diagnostics::AttemptDiagnostics {
    crate::domain::inference::value_objects::attempt_diagnostics::AttemptDiagnostics {
        attempt_number: 1,
        wire_status,
        ..Default::default()
    }
}

/// #2600 review: what the trace says the request did.
#[test]
fn attempt_facts_read_the_trace() {
    use super::attempt_facts;
    use crate::domain::inference::events::request_observation::RequestTrace;
    use crate::domain::inference::services::terminal_guidance::{Output, Response};

    let none = RequestTrace::default();
    none.start();
    let facts = attempt_facts(&none, false);
    assert_eq!((facts.attempts, facts.response), (1, Response::Unknown));

    let unanswered = RequestTrace::default();
    unanswered.start();
    unanswered.record_attempt(record(None));
    unanswered.retry();
    unanswered.record_attempt(record(None));
    let facts = attempt_facts(&unanswered, false);
    assert_eq!((facts.attempts, facts.response), (2, Response::NotReceived));

    // A retry that left no record proves nothing about a response.
    let partly = RequestTrace::default();
    partly.start();
    partly.record_attempt(record(None));
    partly.retry();
    let facts = attempt_facts(&partly, false);
    assert_eq!((facts.attempts, facts.response), (2, Response::Unknown));

    let answered = RequestTrace::default();
    answered.start();
    answered.record_attempt(record(None));
    answered.retry();
    answered.record_attempt(record(Some(503)));
    assert_eq!(attempt_facts(&answered, false).response, Response::Received);

    let shown = attempt_facts(&unanswered, true);
    assert_eq!(
        (shown.output, shown.response),
        (Output::Shown, Response::Received)
    );

    // An OAuth refresh resends once: that is no transient retry.
    let refreshed = RequestTrace::default();
    refreshed.start();
    refreshed.oauth_retry();
    assert_eq!(refreshed.attempts(), 2);
    assert_eq!(attempt_facts(&refreshed, false).attempts, 1);
}
