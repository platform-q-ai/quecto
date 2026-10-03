//! #2436 review round 1: the token totals of every path an attempt can
//! end by — an empty streamed reply, a stream that failed after showing
//! output, an OAuth refresh's resend, Anthropic's cache writes — and the
//! admission refusals that send nothing and so announce nothing.
use super::*;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::inference::request_completion::{
    AgentRequestCounters, RequestCompleted, RequestOutcome, RequestSpend,
};
use crate::domain::provider::StreamEvent;

type Events = Arc<Mutex<Vec<AgentProgressEvent>>>;

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

fn usage(input: u32, output: u32, cached: Option<u32>, written: Option<u32>) -> UsageInfo {
    UsageInfo {
        prompt_tokens: input,
        completion_tokens: output,
        cache_read_tokens: cached,
        cache_write_tokens: written,
        context_tokens: None,
        cost: None,
    }
}

fn reply(usage: UsageInfo) -> LlmResponse {
    let mut reply = text_response("done");
    reply.usage = Some(usage);
    reply
}

/// A streamed reply with nothing visible, whose provider reported usage.
fn empty_reply() -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![],
        usage: Some(usage(500, 300, Some(1000), None)),
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn spend(input: u64, output: u64, cached: Option<u64>) -> Option<RequestSpend> {
    Some(RequestSpend {
        input_tokens: input,
        cached_tokens: cached,
        cache_write_tokens: None,
        output_tokens: output,
    })
}

/// M1: an empty streamed reply is retried as `empty_stream`; the tokens it
/// reported are its attempt's, in the event and the counters.
#[tokio::test]
async fn an_empty_streamed_reply_reports_its_usage_before_the_retry() {
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![StreamEvent::Done(empty_reply())],
        vec![StreamEvent::Done(reply(usage(10, 2, None, None)))],
    ]));
    let (mut agent, events) = agent_on(provider.clone(), true);
    agent
        .run_loop(&mut vec![Message::user("hi")])
        .await
        .expect("the retry answered");
    assert_eq!(provider.request_count(), 2);
    let seen: Vec<_> = completed(&events)
        .iter()
        .map(|c| (c.attempt, c.outcome, c.spend))
        .collect();
    assert_eq!(
        seen,
        [
            (1, RequestOutcome::Error, spend(500, 300, Some(1000))),
            (2, RequestOutcome::Ok, spend(10, 2, None)),
        ]
    );
    assert_eq!(
        agent.request_tally().counters(),
        AgentRequestCounters {
            requests: 2,
            input_tokens: 510,
            cached_tokens: 1000,
            cache_write_tokens: 0,
            output_tokens: 302,
        }
    );
}

/// M1: an empty reply after shown output (here, thinking) is not retried;
/// its usage is still its attempt's.
#[tokio::test]
async fn an_empty_reply_after_shown_output_still_reports_its_usage() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        StreamEvent::ThinkingDelta("hmm".into()),
        StreamEvent::Done(empty_reply()),
    ]]));
    let (mut agent, events) = agent_on(provider.clone(), true);
    assert!(
        agent
            .run_loop(&mut vec![Message::user("hi")])
            .await
            .is_err()
    );
    assert_eq!(provider.request_count(), 1, "never retried after output");
    let seen: Vec<_> = completed(&events)
        .iter()
        .map(|c| (c.request_index, c.attempt, c.outcome, c.spend))
        .collect();
    assert_eq!(
        seen,
        [(1, 1, RequestOutcome::Error, spend(500, 300, Some(1000)))]
    );
    assert_eq!(
        agent.request_tally().counters(),
        AgentRequestCounters {
            requests: 1,
            input_tokens: 500,
            cached_tokens: 1000,
            cache_write_tokens: 0,
            output_tokens: 300,
        }
    );
}

/// L5: a stream that fails after showing output is never retried: one
/// error event, the only request sent.
#[tokio::test]
async fn a_failure_after_shown_output_is_one_error_event() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        StreamEvent::TextDelta("partial".into()),
        StreamEvent::Error("HTTP 503 from Codex".into()),
    ]]));
    let (mut agent, events) = agent_on(provider.clone(), true);
    assert!(
        agent
            .run_loop(&mut vec![Message::user("hi")])
            .await
            .is_err()
    );
    assert_eq!(provider.request_count(), 1);
    let seen: Vec<_> = completed(&events)
        .iter()
        .map(|c| (c.request_index, c.attempt, c.outcome, c.spend))
        .collect();
    assert_eq!(seen, [(1, 1, RequestOutcome::Error, None)]);
}

/// M2: Anthropic's cache writes are their own bucket, carried apart from
/// cache reads and full-price input.
#[tokio::test]
async fn cache_writes_are_counted_apart() {
    let provider = Arc::new(MockProvider::new(vec![reply(usage(
        100,
        8,
        Some(1000),
        Some(5000),
    ))]));
    let (mut agent, events) = agent_on(provider, false);
    agent
        .run_loop(&mut vec![Message::user("hi")])
        .await
        .unwrap();
    assert_eq!(
        completed(&events)[0].spend,
        Some(RequestSpend {
            input_tokens: 100,
            cached_tokens: Some(1000),
            cache_write_tokens: Some(5000),
            output_tokens: 8,
        })
    );
    assert_eq!(
        agent.request_tally().counters(),
        AgentRequestCounters {
            requests: 1,
            input_tokens: 100,
            cached_tokens: 1000,
            cache_write_tokens: 5000,
            output_tokens: 8,
        }
    );
}

/// A provider that answers 401 until its token is refreshed.
#[derive(Debug)]
struct Unauthorized {
    calls: Arc<std::sync::atomic::AtomicU32>,
    fresh: bool,
}

impl LlmProvider for Unauthorized {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "anthropic"
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>>
    {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let fresh = self.fresh;
        Box::pin(async move {
            match fresh {
                true => Ok(reply(usage(40, 4, None, None))),
                false => Err(DomainError::Provider(
                    "provider error (401): unauthorized".into(),
                )),
            }
        })
    }
}

/// L5: an OAuth refresh's resend, through the agent loop, is a request of
/// its own: the refused attempt ends as an error, the resend as ok.
#[tokio::test]
async fn an_oauth_resend_is_its_own_request() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    use crate::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
    let home = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(home.path()));
    store
        .store(Credential {
            provider: "anthropic".into(),
            token: "stale".into(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".into()),
            account_id: None,
        })
        .unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let fresh_calls = calls.clone();
    let refreshable = Arc::new(RefreshableProvider::new(RefreshableConfig {
        inner: Arc::new(Unauthorized {
            calls: calls.clone(),
            fresh: false,
        }),
        store,
        provider_name: "anthropic".into(),
        credential_provider: "anthropic".into(),
        refresh_fn: Arc::new(|_, _| Box::pin(async { Ok("fresh".to_string()) })),
        factory: Arc::new(move |_| {
            Arc::new(Unauthorized {
                calls: fresh_calls.clone(),
                fresh: true,
            }) as Arc<dyn LlmProvider>
        }),
    }));
    let (mut agent, events) = agent_on(refreshable, false);
    agent
        .run_loop(&mut vec![Message::user("hi")])
        .await
        .expect("the resend answered");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let seen: Vec<_> = completed(&events)
        .iter()
        .map(|c| (c.request_index, c.attempt, c.outcome, c.spend))
        .collect();
    assert_eq!(
        seen,
        [
            (1, 1, RequestOutcome::Error, None),
            (2, 2, RequestOutcome::Ok, spend(40, 4, None)),
        ]
    );
    assert_eq!(completed(&events)[1].provider, "anthropic");
}

/// An admission gate that admits the attempts `admit` allows.
#[derive(Debug)]
struct Gate {
    admit: fn(crate::domain::provider::RequestAttempt) -> bool,
}

impl crate::application::providers::ports::RequestAdmission for Gate {
    fn check(
        &self,
        attempt: crate::domain::provider::RequestAttempt,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>> {
        let admitted = (self.admit)(attempt);
        Box::pin(async move {
            match admitted {
                true => Ok(()),
                false => Err(DomainError::Provider("admission refused".into())),
            }
        })
    }
}

/// L5: a request refused before its first attempt sent nothing, so it is
/// no request: nothing is announced or counted.
#[tokio::test]
async fn a_refused_first_attempt_announces_nothing() {
    let provider = Arc::new(MockProvider::new(vec![text_response("never")]));
    let (agent, events) = agent_on(provider.clone(), false);
    let mut agent = agent.with_request_admission(Some(Arc::new(Gate { admit: |_| false })));
    assert!(
        agent
            .run_loop(&mut vec![Message::user("hi")])
            .await
            .is_err()
    );
    assert_eq!(provider.request_count(), 0);
    assert!(completed(&events).is_empty());
    assert_eq!(
        agent.request_tally().counters(),
        AgentRequestCounters::default()
    );
}

/// L5: a refused reattempt sends nothing: only the attempt that failed
/// before it is announced.
#[tokio::test]
async fn a_refused_reattempt_announces_only_the_attempt_before_it() {
    use crate::domain::provider::RequestAttempt;
    let inner = Arc::new(MockProvider::new_results(vec![
        Err(DomainError::Provider("HTTP 503 from upstream".into())),
        Ok(text_response("never")),
    ]));
    let retrying = Arc::new(
        crate::infrastructure::providers::retry::RetryingProvider::new(
            inner.clone(),
            crate::infrastructure::providers::retry::RetryConfig::no_delay(3),
        ),
    );
    let (agent, events) = agent_on(retrying, false);
    let gate = Gate {
        admit: |attempt| matches!(attempt, RequestAttempt::First),
    };
    let mut agent = agent.with_request_admission(Some(Arc::new(gate)));
    assert!(
        agent
            .run_loop(&mut vec![Message::user("hi")])
            .await
            .is_err()
    );
    assert_eq!(inner.request_count(), 1);
    let seen: Vec<_> = completed(&events)
        .iter()
        .map(|c| (c.request_index, c.attempt, c.outcome))
        .collect();
    assert_eq!(seen, [(1, 1, RequestOutcome::Error)]);
    assert_eq!(agent.request_tally().counters().requests, 1);
}
