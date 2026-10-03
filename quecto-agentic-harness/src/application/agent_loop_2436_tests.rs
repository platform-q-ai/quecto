//! #2436: every provider request an agent sends — each attempt, retries
//! included — is announced once as it ends, numbered in the agent's own
//! sequence and counted in its own tally, whatever its provider and
//! whoever else shares its admission.
use super::*;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::inference::request_completion::{
    AgentRequestCounters, RequestCompleted, RequestOutcome, RequestSpend,
};
use crate::domain::provider::StreamEvent;

type Events = Arc<Mutex<Vec<AgentProgressEvent>>>;

/// An agent on `provider` whose progress events are kept.
fn agent_on(provider: Arc<dyn LlmProvider>, streaming: bool) -> (AgentLoopImpl, Events) {
    let events: Events = Arc::default();
    let kept = events.clone();
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        progress_callback: Some(Arc::new(move |event| kept.lock().unwrap().push(event))),
        streaming,
        ..test_config(provider, Box::new(MockRegistry::new()))
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

fn reply(input: u32, cached: Option<u32>, output: u32) -> LlmResponse {
    let mut reply = text_response("done");
    reply.usage = Some(UsageInfo {
        prompt_tokens: input,
        completion_tokens: output,
        cache_read_tokens: cached,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    });
    reply
}

fn server_error() -> DomainError {
    DomainError::Provider("HTTP 503 from upstream: overloaded".into())
}

/// Non-streaming: the retry decorator's second attempt is a request of its
/// own, announced after the first, which ended in error with no usage.
#[tokio::test]
async fn each_attempt_of_a_retried_request_is_its_own_event() {
    let inner = Arc::new(MockProvider::new_results(vec![
        Err(server_error()),
        Ok(reply(120, Some(80), 9)),
    ]));
    let retrying = Arc::new(
        crate::infrastructure::providers::retry::RetryingProvider::new(
            inner.clone(),
            crate::infrastructure::providers::retry::RetryConfig::no_delay(3),
        ),
    );
    let (mut agent, events) = agent_on(retrying, false);
    agent
        .run_loop(&mut vec![Message::user("hi")])
        .await
        .expect("the retry completed the turn");
    let seen = completed(&events);
    assert_eq!(inner.request_count(), 2);
    assert_eq!(seen.len(), 2, "one event per attempt: {seen:?}");
    let first = &seen[0];
    assert_eq!(
        (first.request_index, first.attempt, first.outcome),
        (1, 1, RequestOutcome::Error)
    );
    assert_eq!(first.spend, None, "a failed attempt reported no usage");
    let second = &seen[1];
    assert_eq!(
        (second.request_index, second.attempt, second.outcome),
        (2, 2, RequestOutcome::Ok)
    );
    assert_eq!(
        second.spend,
        Some(RequestSpend {
            input_tokens: 120,
            cached_tokens: Some(80),
            cache_write_tokens: None,
            output_tokens: 9,
        })
    );
    assert_eq!(
        (second.model.as_str(), second.provider.as_str()),
        ("test-model", "mock"),
        "the provider the request was routed to, not a decorator"
    );
    assert_eq!(agent.request_tally().counters().requests, 2);
}

/// Streaming: the loop's own stream re-initiation counts the same way.
#[tokio::test]
async fn a_streaming_retry_is_its_own_event_too() {
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![StreamEvent::Error("HTTP 503 from Codex".into())],
        vec![StreamEvent::Done(reply(10, None, 2))],
    ]));
    let (mut agent, events) = agent_on(provider.clone(), true);
    agent
        .run_loop(&mut vec![Message::user("hi")])
        .await
        .expect("the stream recovered");
    let seen: Vec<_> = completed(&events)
        .iter()
        .map(|c| (c.request_index, c.attempt, c.outcome, c.spend.is_some()))
        .collect();
    assert_eq!(provider.request_count(), 2);
    assert_eq!(
        seen,
        [
            (1, 1, RequestOutcome::Error, false),
            (2, 2, RequestOutcome::Ok, true)
        ]
    );
    assert_eq!(completed(&events)[1].provider, "mock-streaming");
}

/// The tally counts across turns, and sums only what was reported: cached
/// tokens a provider never reports stay zero, not guessed.
#[tokio::test]
async fn the_counters_accumulate_across_turns() {
    let provider = Arc::new(MockProvider::new(vec![
        reply(100, Some(60), 5),
        reply(200, None, 7),
    ]));
    let (mut agent, events) = agent_on(provider, false);
    let mut messages = vec![Message::user("one")];
    agent.run_loop(&mut messages).await.unwrap();
    messages.push(Message::user("two"));
    agent.run_loop(&mut messages).await.unwrap();
    let indices: Vec<_> = completed(&events).iter().map(|c| c.request_index).collect();
    assert_eq!(indices, [1, 2]);
    assert_eq!(
        agent.request_tally().counters(),
        AgentRequestCounters {
            requests: 2,
            input_tokens: 300,
            cached_tokens: 60,
            cache_write_tokens: 0,
            output_tokens: 12,
        }
    );
}

/// A request that fails is announced as an error, with no usage invented.
#[tokio::test]
async fn a_failed_request_is_an_error_event() {
    let provider = Arc::new(MockProvider::new_results(vec![Err(DomainError::Provider(
        "HTTP 429: insufficient_quota".into(),
    ))]));
    let (mut agent, events) = agent_on(provider, false);
    assert!(
        agent
            .run_loop(&mut vec![Message::user("hi")])
            .await
            .is_err()
    );
    let seen = completed(&events);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].outcome, RequestOutcome::Error);
    assert_eq!((seen[0].request_index, seen[0].attempt), (1, 1));
    assert_eq!(seen[0].spend, None);
    assert_eq!(agent.request_tally().counters().input_tokens, 0);
}

/// A provider that never answers.
#[derive(Debug)]
struct Silent;
impl LlmProvider for Silent {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "silent"
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>>
    {
        Box::pin(std::future::pending())
    }
}

/// A request dropped in flight (an abort) is announced as cancelled.
#[tokio::test]
async fn a_dropped_request_is_a_cancelled_event() {
    let (mut agent, events) = agent_on(Arc::new(Silent), false);
    let mut messages = vec![Message::user("hi")];
    {
        let mut run = Box::pin(agent.run_loop(&mut messages));
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(run.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    let seen = completed(&events);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].outcome, RequestOutcome::Cancelled);
    assert_eq!(agent.request_tally().counters().requests, 1);
}

/// One admission gate shared by two loops: it counts every check.
#[derive(Debug, Default)]
struct SharedGroup {
    checks: Mutex<u64>,
}
impl crate::application::providers::ports::RequestAdmission for SharedGroup {
    fn check(
        &self,
        _attempt: crate::domain::provider::RequestAttempt,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>> {
        *self.checks.lock().unwrap() += 1;
        Box::pin(async { Ok(()) })
    }
}

/// Each agent runs in its own process with its own loop and tally; agents
/// in different processes may still share one quota group at the broker.
/// Two loops behind one admission gate — the unit-level stand-in for that
/// — each count only the requests their own loop sent, whatever the gate
/// saw.
#[tokio::test]
async fn each_loop_counts_only_its_own_requests_behind_a_shared_gate() {
    let group = Arc::new(SharedGroup::default());
    let busy = Arc::new(MockProvider::new(vec![reply(10, None, 1); 3]));
    let quiet = Arc::new(MockProvider::new(vec![reply(5, None, 1)]));
    let (busy, busy_events) = agent_on(busy, false);
    let (quiet, quiet_events) = agent_on(quiet, false);
    let mut busy = busy.with_request_admission(Some(group.clone()));
    let mut quiet = quiet.with_request_admission(Some(group.clone()));
    let mut busy_messages = vec![Message::user("loop")];
    for turn in 0..3 {
        if turn > 0 {
            busy_messages.push(Message::user("again"));
        }
        busy.run_loop(&mut busy_messages).await.unwrap();
    }
    quiet
        .run_loop(&mut vec![Message::user("once")])
        .await
        .unwrap();
    assert_eq!(*group.checks.lock().unwrap(), 4, "the gate saw both loops");
    assert_eq!(busy.request_tally().counters().requests, 3);
    assert_eq!(
        quiet.request_tally().counters(),
        AgentRequestCounters {
            requests: 1,
            input_tokens: 5,
            cached_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 1,
        }
    );
    let quiet_indices: Vec<_> = completed(&quiet_events)
        .iter()
        .map(|c| c.request_index)
        .collect();
    assert_eq!(quiet_indices, [1], "its own sequence, not the group's");
    assert_eq!(completed(&busy_events).len(), 3);
}
