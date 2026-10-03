//! #2436 review: an attempt is not sent until admission admits it. One
//! refused, cancelled while it waited, or dropped with its turn while it
//! waited is never announced or counted, and takes no `attempt` number;
//! one that waited reports the wait as `queued_ms` and its duration from
//! its admission.
use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::agent_turn::ports::AgentLoop;
use crate::application::ports::AttemptAcquisition;
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::agent::AgentProgressEvent;
use crate::domain::inference::request_completion::{RequestCompleted, RequestOutcome};
use crate::domain::message::Message;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug)]
struct Permit;
impl AttemptPermit for Permit {
    fn feedback(&mut self, _: ThrottleFeedback) {}
    fn finish(self: Box<Self>, _: Feedback) {}
}

/// How the gate answers each acquire, in turn; past the list it admits.
#[derive(Debug, Clone, Copy)]
enum Answer {
    /// Never admits; says it was asked.
    Wait,
    /// Refuses with a retryable broker error.
    Refuse,
    /// Admits after a wait of this many milliseconds.
    AdmitAfter(u64),
}

#[derive(Debug)]
struct Gate {
    answers: Vec<Answer>,
    asked: Arc<tokio::sync::Notify>,
    calls: AtomicU32,
}

impl Gate {
    fn new(answers: Vec<Answer>) -> Arc<Self> {
        Arc::new(Self {
            answers,
            asked: Arc::default(),
            calls: AtomicU32::new(0),
        })
    }
}

impl AttemptAdmission for Gate {
    fn acquire(&self) -> AttemptAcquisition<'_> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) as usize;
        let answer = self.answers.get(call).copied();
        self.asked.notify_one();
        Box::pin(async move {
            match answer {
                Some(Answer::Wait) => std::future::pending().await,
                Some(Answer::Refuse) => Err(DomainError::Provider(
                    "HTTP 503: admission group unavailable".into(),
                )),
                Some(Answer::AdmitAfter(ms)) => {
                    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    Ok(Box::new(Permit) as Box<dyn AttemptPermit>)
                }
                None => Ok(Box::new(Permit) as Box<dyn AttemptPermit>),
            }
        })
    }
}

fn reply() -> LlmResponse {
    LlmResponse {
        content: Some("done".into()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

/// A provider whose every attempt goes through `gate`, as a gated adapter's
/// does: in the request's own future (`chat`), or in a task of its own
/// (`chat_stream_incremental`).
#[derive(Debug)]
struct Gated(Arc<Gate>);

impl LlmProvider for Gated {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "gated"
    }
    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let gate: Arc<dyn AttemptAdmission> = self.0.clone();
        let trace = request.trace.clone();
        Box::pin(async move {
            run(&gate, trace, None, None, |_| async { Ok(reply()) })
                .await
                .map_err(AttemptError::into_domain)
        })
    }
    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        let gate: Arc<dyn AttemptAdmission> = self.0.clone();
        let trace = request.trace.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            queue_before_spawn(Some(&gate), trace.as_ref());
            tokio::spawn(async move {
                let sent = run(&gate, trace, None, Some(&tx), |_| async { Ok(()) }).await;
                let event = match sent {
                    Ok(()) => StreamEvent::Done(reply()),
                    Err(error) => StreamEvent::Error(error.into_domain().to_string()),
                };
                let _ = tx.send(event).await;
            });
            rx
        })
    }
}

type Events = Arc<Mutex<Vec<AgentProgressEvent>>>;

fn agent_on(provider: Arc<dyn LlmProvider>, streaming: bool) -> (AgentLoopImpl, Events) {
    let events: Events = Arc::default();
    let kept = events.clone();
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "m".into(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: "cli:test".into(),
        max_context_tokens: 190_000,
        progress_callback: Some(Arc::new(move |event| kept.lock().unwrap().push(event))),
        streaming,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    (agent, events)
}

fn completed(events: &Events) -> Vec<RequestCompleted> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentProgressEvent::RequestCompleted(completed) => Some(completed.clone()),
            _ => None,
        })
        .collect()
}

/// Production order: the turn is dropped (an abort) while its attempt
/// waits in a gate that never admits it.
async fn drop_the_turn_while_queued(streaming: bool) {
    let gate = Gate::new(vec![Answer::Wait]);
    let (mut agent, events) = agent_on(Arc::new(Gated(gate.clone())), streaming);
    let mut messages = vec![Message::user("hi")];
    {
        let mut turn = agent.process(&mut messages);
        tokio::select! {
            _ = &mut turn => panic!("the gate never admits"),
            _ = gate.asked.notified() => {}
        }
    }
    // Let a streaming attempt's own task see its receiver close.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(
        completed(&events).is_empty(),
        "nothing was sent: {:?}",
        completed(&events)
    );
    assert_eq!(agent.request_tally().counters().requests, 0);
}

#[tokio::test]
async fn a_turn_dropped_while_queued_announces_nothing_streaming() {
    drop_the_turn_while_queued(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_dropped_while_queued_announces_nothing_streaming_multi_thread() {
    drop_the_turn_while_queued(true).await;
}

#[tokio::test]
async fn a_turn_dropped_while_queued_announces_nothing_non_streaming() {
    drop_the_turn_while_queued(false).await;
}

/// L1: an attempt refused with a retryable broker error, then sent, is the
/// request's first sent attempt: `attempt: 1`, `requestIndex: 1`.
#[tokio::test]
async fn a_refused_attempt_takes_no_attempt_number() {
    let gate = Gate::new(vec![Answer::Refuse]);
    let retrying = Arc::new(
        crate::infrastructure::providers::retry::RetryingProvider::new(
            Arc::new(Gated(gate.clone())),
            crate::infrastructure::providers::retry::RetryConfig::no_delay(3),
        ),
    );
    let (mut agent, events) = agent_on(retrying, false);
    agent
        .process(&mut vec![Message::user("hi")])
        .await
        .expect("the second attempt was admitted and answered");
    assert_eq!(gate.calls.load(Ordering::SeqCst), 2);
    let seen: Vec<_> = completed(&events)
        .iter()
        .map(|c| (c.request_index, c.attempt, c.outcome))
        .collect();
    assert_eq!(seen, [(1, 1, RequestOutcome::Ok)]);
}

/// L2: an attempt that waited for admission reports the wait as
/// `queued_ms`, and its duration only from its admission.
#[tokio::test]
async fn an_admission_wait_is_queued_ms_not_duration() {
    let gate = Gate::new(vec![Answer::AdmitAfter(60)]);
    let (mut agent, events) = agent_on(Arc::new(Gated(gate)), false);
    agent
        .process(&mut vec![Message::user("hi")])
        .await
        .expect("admitted");
    let seen = completed(&events);
    assert_eq!(seen.len(), 1);
    let queued = seen[0].queued_ms.expect("it waited for admission");
    assert!(queued >= 60, "waited {queued} ms");
    assert!(
        seen[0].duration_ms < queued,
        "duration {} ms runs from admission, not the {queued} ms wait",
        seen[0].duration_ms
    );
}

/// The turn is dropped before the attempt's own task first runs: it was
/// marked as waiting before the spawn, so it is still withdrawn.
#[tokio::test]
async fn a_turn_dropped_before_its_attempt_task_runs_announces_nothing() {
    let gate = Gate::new(vec![Answer::Wait]);
    let (mut agent, events) = agent_on(Arc::new(Gated(gate.clone())), true);
    let mut messages = vec![Message::user("hi")];
    {
        let mut turn = agent.process(&mut messages);
        std::future::poll_fn(|cx| {
            assert!(turn.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }
    assert_eq!(gate.calls.load(Ordering::SeqCst), 0, "its task had not run");
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(completed(&events).is_empty(), "{:?}", completed(&events));
    assert_eq!(agent.request_tally().counters().requests, 0);
}
