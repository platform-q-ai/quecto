use super::*;

#[tokio::test]
async fn successful_usage_is_retained_when_a_later_provider_request_fails() {
    let (mut agent, _) = make_agent(vec![], vec![("read", "content")]);
    agent.provider = Arc::new(MockProvider::new_results(vec![
        Ok(tool_call_response_with_usage(
            "read",
            r#"{"path":"x"}"#,
            UsageFixture(10, 2, 3, 4, 1000),
        )),
        Err(DomainError::Provider("HTTP 429: insufficient_quota".into())),
    ]));
    assert!(
        agent
            .run_loop(&mut vec![Message::user("read")])
            .await
            .is_err()
    );
    let usage = agent.take_unreported_usage();
    assert_eq!(usage.billed_input_tokens, 10);
    assert_eq!(usage.billed_output_tokens, 2);
    assert_eq!(usage.cache_read_tokens, 3);
    assert_eq!(usage.cost_micro_usd, 1000);
    assert_eq!(agent.take_unreported_usage().billed_input_tokens, 0);
}

#[tokio::test]
async fn request_diagnostics_distinguish_missing_usage_from_zero_and_retain_failures() {
    let (mut agent, _) = make_agent(vec![], vec![("read", "content")]);
    agent.provider = Arc::new(MockProvider::new_results(vec![
        Ok(tool_call_response("read", r#"{"path":"x"}"#)),
        Err(DomainError::Provider("HTTP 429: insufficient_quota".into())),
    ]));
    assert!(
        agent
            .run_loop(&mut vec![Message::user("read")])
            .await
            .is_err()
    );
    let records = agent.take_request_observations();
    assert_eq!(
        records.len(),
        2,
        "every logical request needs a diagnostic record"
    );
    assert_eq!(records[0].input_tokens, None);
    assert!(records[0].estimated_context_tokens > 0);
    assert_eq!(records[0].outcome, "succeeded");
    assert_eq!(records[1].outcome, "failed");
    assert_eq!(records[1].instrumented_attempts, 1);
    assert_eq!(records[1].harness_prefix_unchanged, Some(true));
    assert_eq!(agent.take_unreported_usage().billed_input_tokens, 0);
}

struct ReportOnlyAdmission;
impl crate::domain::tool::ToolExecutionAdmission for ReportOnlyAdmission {
    fn check<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + 'a>> {
        Box::pin(async { Err(DomainError::Tool("swarm terminal: report only".into())) })
    }
}

#[tokio::test]
async fn terminal_report_cannot_execute_mutating_tools() {
    let (agent, _) = make_agent(
        vec![tool_call_response("bash", "{}"), text_response("report")],
        vec![("bash", "MUTATION_EXECUTED")],
    );
    let mut agent = agent.with_tool_admission(Some(Arc::new(ReportOnlyAdmission)));
    let mut messages = vec![Message::user("report")];
    agent.run_loop(&mut messages).await.unwrap();
    assert!(
        messages
            .iter()
            .any(|message| message.role == Role::Tool && message.content.contains("report only"))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.content != "MUTATION_EXECUTED")
    );
}

#[derive(Default)]
struct RetryAccounting {
    records: Mutex<Vec<String>>,
}
impl crate::domain::request_observation::RequestAccounting for RetryAccounting {
    fn record<'a>(
        &'a self,
        observation: &'a crate::domain::request_observation::RequestObservation,
    ) -> crate::domain::subagent_launch::LaunchFuture<'a, Result<(), DomainError>> {
        Box::pin(async move {
            let mut records = self.records.lock().unwrap();
            records.push(observation.request_id.clone());
            if records.len() == 1 {
                // Store contention keeps the record pending for a retry.
                Err(DomainError::Tool("database is locked".into()))
            } else {
                Ok(())
            }
        })
    }
}
#[tokio::test]
async fn failed_accounting_is_retried_with_original_observation_id() {
    let (agent, _) = make_agent(vec![text_response("paid response")], vec![]);
    let accounting = Arc::new(RetryAccounting::default());
    let mut agent = agent.with_request_accounting(Some(accounting.clone()));
    assert!(
        agent
            .run_loop(&mut vec![Message::user("report")])
            .await
            .is_err()
    );
    let records = agent.take_request_observations();
    assert_eq!(records[0].outcome, "succeeded");
    agent.flush_request_accounting().await.unwrap();
    agent.flush_request_accounting().await.unwrap();
    assert_eq!(
        *accounting.records.lock().unwrap(),
        vec![records[0].request_id.clone(); 2]
    );
}

#[derive(Default)]
struct RejectingAccounting {
    records: Mutex<Vec<String>>,
}
impl crate::domain::request_observation::RequestAccounting for RejectingAccounting {
    fn record<'a>(
        &'a self,
        observation: &'a crate::domain::request_observation::RequestObservation,
    ) -> crate::domain::subagent_launch::LaunchFuture<'a, Result<(), DomainError>> {
        Box::pin(async move {
            self.records
                .lock()
                .unwrap()
                .push(observation.request_id.clone());
            Err(DomainError::Tool(
                "request diagnostic ledger full; export before starting another run".into(),
            ))
        })
    }
}
/// Diagnostics never block inference: a durable store rejection drops the
/// record with a warning and the next turn (e.g. the terminal report) runs.
#[tokio::test]
async fn durable_accounting_rejection_drops_the_record_without_blocking_turns() {
    let (agent, _) = make_agent(
        vec![text_response("first"), text_response("report")],
        vec![],
    );
    let accounting = Arc::new(RejectingAccounting::default());
    let mut agent = agent.with_request_accounting(Some(accounting.clone()));
    agent
        .run_loop(&mut vec![Message::user("go")])
        .await
        .expect("a rejected diagnostic must not fail the turn");
    agent
        .run_loop(&mut vec![Message::user("report")])
        .await
        .expect("the report turn still runs");
    agent.flush_request_accounting().await.unwrap();
    let attempted = accounting.records.lock().unwrap().clone();
    assert_eq!(
        attempted.len(),
        2,
        "each record was offered exactly once, then dropped"
    );
    assert_ne!(attempted[0], attempted[1]);
}

#[derive(Debug)]
struct PendingProvider;
impl LlmProvider for PendingProvider {
    fn name(&self) -> &str {
        "pending"
    }
    fn chat(
        &self,
        _: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        Box::pin(std::future::pending())
    }
}
#[tokio::test]
async fn cancelled_provider_request_remains_in_accounting_outbox() {
    let (mut agent, _) = make_agent(vec![], vec![]);
    agent.provider = Arc::new(PendingProvider);
    let accounting = Arc::new(RetryAccounting::default());
    agent = agent.with_request_accounting(Some(accounting.clone()));
    let mut messages = vec![Message::user("report")];
    {
        let mut run = Box::pin(agent.run_loop(&mut messages));
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(run.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    let records = agent.take_request_observations();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, "cancelled");
    assert_eq!(records[0].instrumented_attempts, 1);
    assert!(agent.flush_request_accounting().await.is_err());
    agent.flush_request_accounting().await.unwrap();
    assert_eq!(
        *accounting.records.lock().unwrap(),
        vec![records[0].request_id.clone(); 2]
    );
}

#[tokio::test]
async fn mock_streaming_provider_trait_surface_chat_and_incremental() {
    let provider =
        MockStreamingProvider::new(vec![vec![crate::domain::provider::StreamEvent::Done(
            text_response("done"),
        )]]);
    let messages = [];
    let tools = [];
    let request = ChatRequest {
        trace: None,
        admission: None,
        messages: &messages,
        tools: &tools,
        model: "test-model",
        max_tokens: 9,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    assert_eq!(provider.name(), "mock-streaming");
    assert!(provider.as_any().is::<()>());
    let mut rx = provider.chat_stream_incremental(request).await;
    assert!(matches!(
        rx.recv().await,
        Some(crate::domain::provider::StreamEvent::Done(_))
    ));
    assert_eq!(provider.request_count(), 1);

    let provider =
        MockStreamingProvider::new(vec![vec![crate::domain::provider::StreamEvent::Done(
            text_response("chat done"),
        )]]);
    let messages = [];
    let tools = [];
    let request = ChatRequest {
        trace: None,
        admission: None,
        messages: &messages,
        tools: &tools,
        model: "test-model",
        max_tokens: 9,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let response = provider.chat(request).await.unwrap();
    assert_eq!(response.content.as_deref(), Some("chat done"));
}
