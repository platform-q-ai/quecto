//! #2210: the loop caps every request's attempts at its output limit's worth
//! of bytes, never retries a reply stopped at that cap, shows the request in
//! flight to `get_state`, and records a request it leaves in flight — with
//! the attempt it interrupted — for the audit log exactly once.
use super::*;
use crate::application::audit::ports::AuditSink;
use crate::domain::attempt_diagnostics::Termination;
use crate::domain::audit::AuditEvent;
use crate::domain::provider::StreamEvent;
use crate::domain::request_progress::OutputCapped;
use std::time::Instant;

#[derive(Debug, Default)]
struct RecordingAudit {
    events: Mutex<Vec<AuditEvent>>,
}

impl AuditSink for RecordingAudit {
    fn emit(
        &self,
        _turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            self.events.lock().unwrap().push(event);
            Ok(())
        })
    }
}

impl RecordingAudit {
    fn request_observations(&self) -> Vec<crate::domain::request_observation::RequestObservation> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                AuditEvent::RequestObserved { observation } => Some((**observation).clone()),
                _ => None,
            })
            .collect()
    }
}

/// How the probe's reply goes, after it streamed `delta` once.
#[derive(Debug, Clone, Copy)]
enum Reply {
    /// It is stopped at an output cap of 64 bytes.
    Capped,
    /// It never ends.
    Hangs,
}

/// A streaming provider that acts as an observed transport does: it
/// follows its attempt on the request's trace, and reports the cap it saw.
#[derive(Debug)]
struct Probe {
    reply: Reply,
    requests: Mutex<Vec<Option<u64>>>,
}

impl Probe {
    fn new(reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn caps(&self) -> Vec<Option<u64>> {
        self.requests.lock().unwrap().clone()
    }
}

impl LlmProvider for Probe {
    fn name(&self) -> &str {
        "probe"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        Box::pin(async { Err(DomainError::Provider("streams only".into())) })
    }

    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<
        Box<dyn std::future::Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>,
    > {
        let trace = request
            .trace
            .clone()
            .expect("the loop traces every request");
        self.requests.lock().unwrap().push(trace.output_cap());
        let reply = self.reply;
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            tokio::spawn(async move {
                trace.begin_attempt(trace.attempts().max(1), Instant::now(), 1);
                trace.observe_event(Instant::now(), 5, true);
                match reply {
                    Reply::Capped => {
                        let capped = OutputCapped { cap: 64 };
                        let _ = tx.send(StreamEvent::Error(capped.to_string())).await;
                    }
                    Reply::Hangs => {
                        let _ = tx.send(StreamEvent::TextDelta("hello".into())).await;
                        std::future::pending::<()>().await;
                    }
                }
            });
            rx
        })
    }
}

fn streaming_agent(provider: Arc<Probe>, audit: Arc<RecordingAudit>) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        streaming: true,
        audit_log: Some(audit),
        ..test_config(provider, Box::new(MockRegistry::new()))
    })
    .with_max_tool_iterations(1)
}

#[tokio::test]
async fn every_request_is_capped_at_its_output_limits_worth_of_bytes() {
    for (declared, cap) in [
        (Some(128_000), 1_024_000),
        (
            None,
            8 * u64::from(crate::domain::request_progress::FALLBACK_OUTPUT_TOKENS),
        ),
    ] {
        let provider = Probe::new(Reply::Capped);
        let mut agent =
            streaming_agent(provider.clone(), Arc::default()).with_model_max_tokens(declared);
        let _ = agent.process(&mut vec![Message::user("hi")]).await;
        assert_eq!(provider.caps(), [Some(cap)], "{declared:?}");
    }
}

#[tokio::test]
async fn a_reply_stopped_at_its_output_cap_is_not_retried() {
    let provider = Probe::new(Reply::Capped);
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = streaming_agent(provider.clone(), audit.clone());
    let error = agent
        .process(&mut vec![Message::user("hi")])
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(provider.caps().len(), 1, "one attempt, never retried");
    assert!(error.contains("output cap exceeded: "), "{error}");
    assert!(error.contains("Output cap:"), "guidance: {error}");
    let events = audit.events.lock().unwrap().clone();
    assert!(
        events.iter().any(|event| matches!(
            event,
            AuditEvent::ProviderError { class, .. } if class.as_str() == "output_capped"
        )),
        "{events:?}"
    );
    let observed = audit.request_observations();
    assert_eq!(observed.len(), 1);
    assert_eq!(
        observed[0].error_class,
        Some(crate::domain::provider_error::ProviderErrorClass::OutputCapped)
    );
}

/// `get_state` sees the request while the model streams; once the turn is
/// dropped where it stands, the request is gone from it and its record —
/// with the attempt it interrupted — is audited exactly once.
#[tokio::test]
async fn a_request_left_in_flight_is_shown_live_then_audited_once() {
    let provider = Probe::new(Reply::Hangs);
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = streaming_agent(provider, audit.clone());
    let in_flight = agent.request_in_flight();
    assert!(in_flight.snapshot(Instant::now()).is_none());
    let watched = {
        let in_flight = in_flight.clone();
        async move {
            loop {
                let streamed = in_flight
                    .snapshot(Instant::now())
                    .and_then(|turn| turn.attempt)
                    .filter(|attempt| attempt.output_bytes == 5);
                if let Some(attempt) = streamed {
                    return attempt;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        }
    };
    let mut messages = vec![Message::user("hi")];
    let attempt = tokio::select! {
        _ = agent.process(&mut messages) => panic!("the reply never ends"),
        attempt = tokio::time::timeout(std::time::Duration::from_secs(10), watched) => {
            attempt.expect("the request in flight was shown")
        }
    };
    assert_eq!((attempt.number, attempt.events), (1, 1));
    assert!(attempt.since_last_event_ms.is_some());
    assert!(in_flight.snapshot(Instant::now()).is_none());
    assert!(
        audit.request_observations().is_empty(),
        "not before the turn"
    );

    agent.audit_interrupted_requests().await;
    agent.audit_interrupted_requests().await;
    let observed = audit.request_observations();
    assert_eq!(observed.len(), 1, "audited exactly once");
    assert_eq!(observed[0].outcome, "cancelled");
    let interrupted = observed[0].attempt_diagnostics.last().unwrap();
    assert_eq!(interrupted.termination, Termination::Interrupted);
    assert_eq!((interrupted.event_count, interrupted.output_bytes), (1, 5));
    assert!(interrupted.first_token_ms.is_some());
}

/// A deadline's settlement writes the interrupted request once, even after
/// a turn boundary already drained the queue.
#[tokio::test]
async fn a_stopped_run_audits_each_interrupted_request_once() {
    let provider = Probe::new(Reply::Hangs);
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = streaming_agent(provider, audit.clone());
    let mut messages = vec![Message::user("hi")];
    let stopped = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        agent.process(&mut messages),
    )
    .await;
    assert!(stopped.is_err(), "the run was stopped at its deadline");
    agent
        .settle_stopped_run("max-time 1s exceeded")
        .await
        .unwrap();
    agent
        .settle_stopped_run("max-time 1s exceeded")
        .await
        .unwrap();
    assert_eq!(audit.request_observations().len(), 1);
}

/// #2210 review: a non-streaming request (every `chat`, as one-shot agents
/// send) runs its transport inside the request, which drops it first; a run
/// stopped at its deadline still records that attempt as `Interrupted`,
/// with what it had read, gated or not.
#[tokio::test]
async fn a_stopped_non_streaming_request_records_its_attempt_as_interrupted() {
    use crate::infrastructure::providers::stream_idle::tests::{LIVE, servers};
    use crate::infrastructure::providers::stream_idle_provider_tests::Vendor;
    let codex = "data: {\"type\":\"response.created\",\"response\":{}}\n\n\
                 data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n";
    let openai = "{\"choices\":";
    for (vendor, sent, events, output) in
        [(Vendor::Codex, codex, 2, 5), (Vendor::OpenAi, openai, 0, 0)]
    {
        for gated in [false, true] {
            let url = servers::silent_after(sent).await;
            let audit = Arc::new(RecordingAudit::default());
            let mut agent = AgentLoopImpl::new(AgentLoopConfig {
                streaming: false,
                audit_log: Some(audit.clone()),
                ..test_config(
                    vendor.provider(url, gated, LIVE),
                    Box::new(MockRegistry::new()),
                )
            })
            .with_max_tool_iterations(1);
            let mut messages = vec![Message::system("sys"), Message::user("hi")];
            let stopped = tokio::time::timeout(
                std::time::Duration::from_millis(400),
                agent.process(&mut messages),
            )
            .await;
            assert!(stopped.is_err(), "{vendor:?} gated={gated}: {stopped:?}");
            agent.settle_stopped_run("max-time exceeded").await.unwrap();
            let observed = audit.request_observations();
            assert_eq!(observed.len(), 1, "{vendor:?} gated={gated}");
            assert_eq!(observed[0].outcome, "cancelled");
            let attempts = &observed[0].attempt_diagnostics;
            assert_eq!(attempts.len(), 1, "{vendor:?} gated={gated}: {attempts:?}");
            assert_eq!(
                attempts[0].termination,
                Termination::Interrupted,
                "{vendor:?} gated={gated}"
            );
            assert_eq!(
                (attempts[0].event_count, attempts[0].output_bytes),
                (events, output),
                "{vendor:?} gated={gated}"
            );
        }
    }
}

/// #2210 review: an interrupted request is audited under the turn it was
/// sent in.
#[tokio::test]
async fn an_interrupted_request_is_audited_under_its_turn() {
    #[derive(Debug, Default)]
    struct Turns(Mutex<Vec<(u32, AuditEvent)>>);
    impl AuditSink for Turns {
        fn emit(
            &self,
            turn: u32,
            event: AuditEvent,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>>
        {
            Box::pin(async move {
                self.0.lock().unwrap().push((turn, event));
                Ok(())
            })
        }
    }
    let audit = Arc::new(Turns::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        streaming: true,
        audit_log: Some(audit.clone()),
        ..test_config(Probe::new(Reply::Hangs), Box::new(MockRegistry::new()))
    });
    let stopped = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        agent.process(&mut vec![Message::user("hi")]),
    )
    .await;
    assert!(stopped.is_err());
    agent.audit_interrupted_requests().await;
    let turns: Vec<u32> = audit
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, event)| matches!(event, AuditEvent::RequestObserved { .. }))
        .map(|(turn, _)| *turn)
        .collect();
    assert_eq!(turns, [1]);
}
