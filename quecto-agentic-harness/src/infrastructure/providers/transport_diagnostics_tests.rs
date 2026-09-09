#[cfg(test)]
mod sanitization_tests {
    use super::super::*;
    #[test]
    fn headers_are_allowlisted_hashed_and_bounded() {
        let h = headers([
            ("authorization", "SECRET"),
            ("x-request-id", "SECRET"),
            ("retry-after", "123"),
            ("retry-after-ms", "999999999999999999"),
            ("x-ratelimit-reset-tokens", "secret"),
        ]);
        assert_eq!(h.len(), 2);
        let json = serde_json::to_string(&h).unwrap();
        assert!(!json.contains("SECRET"));
        assert_eq!(h[1].value, HeaderValue::Number(123));
    }
    #[test]
    fn typed_fields_never_copy_unknown_values_or_messages() {
        let mut d = AttemptDiagnostics::default();
        typed(
            &mut d,
            &serde_json::json!({"error":{"code":"SECRET","message":"SECRET"}, "response":{"incomplete_details":{"reason":"SECRET"}}}),
        );
        assert_eq!(d.error_code, None);
        assert_eq!(d.incomplete_reason, None);
        typed(
            &mut d,
            &serde_json::json!({"error":{"code":"rate_limit_exceeded"}}),
        );
        assert_eq!(d.error_code, Some(ErrorCode::RateLimitExceeded));
    }
}
#[cfg(test)]
mod transport_tests {
    use super::super::super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    #[derive(Debug)]
    struct Gate;
    #[derive(Debug)]
    struct Permit;
    impl AttemptPermit for Permit {
        fn feedback(&mut self, _: ThrottleFeedback) {}
        fn finish(self: Box<Self>, _: Feedback) {}
    }
    impl AttemptAdmission for Gate {
        fn acquire(&self) -> crate::application::inference_attempt::AttemptAcquisition<'_> {
            Box::pin(async { Ok(Box::new(Permit) as Box<dyn AttemptPermit>) })
        }
    }
    #[derive(Debug)]
    struct DeadlineGate;
    #[derive(Debug)]
    struct DeadlinePermit;
    impl AttemptPermit for DeadlinePermit {
        fn feedback(&mut self, _: ThrottleFeedback) {}
        fn finish(self: Box<Self>, _: Feedback) {}
        fn deadline_expired(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
            Box::pin(async {})
        }
    }
    impl AttemptAdmission for DeadlineGate {
        fn acquire(&self) -> crate::application::inference_attempt::AttemptAcquisition<'_> {
            Box::pin(async { Ok(Box::new(DeadlinePermit) as Box<dyn AttemptPermit>) })
        }
    }
    #[test]
    fn protocol_counts_output_without_retaining_it_and_bounds_lines() {
        let receipt = Receipt(Arc::new(Mutex::new(State {
            permit: Some(Box::new(Permit)),
            failure: false,
            hinted: false,
            trace: None,
            diagnostics: Default::default(),
            started: std::time::Instant::now(),
        })));
        let mut observer = LineObserver {
            carry: Vec::new(),
            oversized: false,
            protocol: ProtocolObserver::new(Profile::new(
                Vendor::Codex,
                crate::infrastructure::providers::attempt_profile::Surface::Assembled,
            )),
        };
        observer.push(
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"SECRET\"}\n",
            &receipt,
        );
        observer.push(b"data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n", &receipt);
        observer.push(
            &vec![b'x'; crate::infrastructure::providers::sse_common::MAX_SSE_LINE_BYTES + 1],
            &receipt,
        );
        observer.finish(&receipt);
        let state = receipt.0.lock().unwrap();
        assert!(state.diagnostics.generated_text);
        assert_eq!(state.diagnostics.event_count, 2);
        assert_eq!(state.diagnostics.oversized_lines, 1);
        assert_eq!(
            state.diagnostics.incomplete_reason,
            Some(IncompleteReason::MaxOutputTokens)
        );
        assert!(
            !serde_json::to_string(&state.diagnostics)
                .unwrap()
                .contains("SECRET")
        );
    }
    #[tokio::test]
    async fn stops_are_recorded_before_owner_release() {
        let trace = Arc::new(RequestTrace::default());
        let gate: Arc<dyn AttemptAdmission> = Arc::new(DeadlineGate);
        let result = run(&gate, Some(trace.clone()), None, None, |_| {
            std::future::pending::<Result<(), DomainError>>()
        })
        .await;
        assert!(result.is_err());
        assert_eq!(
            trace.attempt_diagnostics()[0].termination,
            Termination::Deadline
        );
        let gate: Arc<dyn AttemptAdmission> = Arc::new(Gate);
        let flag = CancelFlag::new();
        let result = run(&gate, Some(trace.clone()), Some(&flag), None, |_| {
            flag.cancel();
            std::future::pending::<Result<(), DomainError>>()
        })
        .await;
        assert!(result.is_err());
        assert_eq!(
            trace.attempt_diagnostics()[1].termination,
            Termination::Cancelled
        );
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let result = run(&gate, Some(trace.clone()), None, Some(&tx), |_| {
            drop(rx);
            std::future::pending::<Result<(), DomainError>>()
        })
        .await;
        assert!(result.is_err());
        assert_eq!(
            trace.attempt_diagnostics()[2].termination,
            Termination::ReceiverClosed
        );
    }
    #[tokio::test]
    async fn forwarding_preserves_first_terminal_and_suppresses_later_deltas() {
        let receipt = Receipt(Arc::new(Mutex::new(State {
            permit: Some(Box::new(Permit)),
            failure: false,
            hinted: false,
            trace: None,
            diagnostics: Default::default(),
            started: std::time::Instant::now(),
        })));
        let (local, rx) = tokio::sync::mpsc::channel(3);
        let (tx, mut output) = tokio::sync::mpsc::channel(3);
        local
            .send(StreamEvent::Error("first".into()))
            .await
            .unwrap();
        local
            .send(StreamEvent::TextDelta("must not forward".into()))
            .await
            .unwrap();
        local
            .send(StreamEvent::Error("second".into()))
            .await
            .unwrap();
        drop(local);
        let terminal = forward(rx, &tx, &receipt).await;
        assert!(matches!(terminal, Some(StreamEvent::Error(e)) if e == "first"));
        assert!(output.try_recv().is_err());
    }
    #[tokio::test]
    async fn done_only_output_sets_presence_without_retaining_content() {
        let receipt = Receipt(Arc::new(Mutex::new(State {
            permit: Some(Box::new(Permit)),
            failure: false,
            hinted: false,
            trace: None,
            diagnostics: Default::default(),
            started: std::time::Instant::now(),
        })));
        let (local, rx) = tokio::sync::mpsc::channel(1);
        let (tx, _) = tokio::sync::mpsc::channel(1);
        local
            .send(StreamEvent::Done(LlmResponse {
                content: Some("SECRET".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: Some(crate::domain::message::StopReason::Unknown("SECRET".into())),
                thinking_blocks: vec![],
            }))
            .await
            .unwrap();
        drop(local);
        forward(rx, &tx, &receipt).await;
        let state = receipt.0.lock().unwrap();
        assert!(state.diagnostics.generated_text);
        assert_eq!(
            state.diagnostics.stop_reason,
            Some(TerminalStopReason::Unknown)
        );
        assert!(
            !serde_json::to_string(&state.diagnostics)
                .unwrap()
                .contains("SECRET")
        );
    }
    struct DoneHandler;
    impl SseHandler for DoneHandler {
        async fn process_line(&mut self, _: &str, tx: &Sender) -> SseLineOutcome {
            tx.send(StreamEvent::Done(LlmResponse {
                content: None,
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            }))
            .await
            .unwrap();
            SseLineOutcome::Done
        }
        async fn on_eof(&mut self, _: &Sender) {}
    }
    #[tokio::test]
    async fn terminal_is_visible_only_after_diagnostics_are_published() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("data: [DONE]\n"))
            .mount(&server)
            .await;
        let trace = Arc::new(RequestTrace::default());
        let gate: Arc<dyn AttemptAdmission> = Arc::new(Gate);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let saved = trace.clone();
        let task = tokio::spawn(async move {
            stream(
                &gate,
                (Some(saved), None),
                reqwest::Client::new().get(server.uri()),
                Profile::new(
                    Vendor::Codex,
                    crate::infrastructure::providers::attempt_profile::Surface::Incremental,
                ),
                tx,
                DoneHandler,
            )
            .await;
        });
        assert!(matches!(rx.recv().await, Some(StreamEvent::Done(_))));
        assert_eq!(trace.attempt_diagnostics().len(), 1);
        assert_eq!(
            trace.attempt_diagnostics()[0].termination,
            Termination::Completed
        );
        task.await.unwrap();
    }
    #[tokio::test]
    async fn wire_facts_are_distinct_from_empty_semantics_and_redacted() {
        for (status, body, terminal, malformed) in [
            (
                200,
                "data: {\"type\":\"response.completed\",\"response\":{\"output\":[]}}\n",
                Some(TerminalEvent::ResponseCompleted),
                0,
            ),
            (200, "data: [DONE]\n", Some(TerminalEvent::Done), 0),
            (200, "data: SECRET\ndata: {\"type\":\"SECRET\"}\n", None, 1),
            (
                503,
                "{\"error\":{\"code\":\"server_error\",\"message\":\"SECRET\"}}",
                None,
                0,
            ),
            (
                429,
                "{\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"SECRET\"}}",
                None,
                0,
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("x-request-id", "SECRET")
                        .insert_header("authorization", "SECRET")
                        .insert_header("retry-after", "2")
                        .set_body_string(body),
                )
                .mount(&server)
                .await;
            let trace = Arc::new(RequestTrace::default());
            let gate: Arc<dyn AttemptAdmission> = Arc::new(Gate);
            let _ = assembled(
                &gate,
                Some(trace.clone()),
                None,
                reqwest::Client::new().get(server.uri()),
                Profile::new(
                    Vendor::Codex,
                    crate::infrastructure::providers::attempt_profile::Surface::Assembled,
                ),
                |_| Ok(()),
            )
            .await;
            let records = trace.attempt_diagnostics();
            assert_eq!(records.len(), 1);
            let d = &records[0];
            assert_eq!(d.wire_status, Some(status));
            assert_eq!(
                d.termination,
                if status == 200 {
                    Termination::Eof
                } else {
                    Termination::HttpError
                }
            );
            assert_eq!(d.terminal_event, terminal);
            assert_eq!(d.parse_errors, malformed);
            assert!(d.finished_unix_ms >= d.started_unix_ms);
            assert!(!serde_json::to_string(d).unwrap().contains("SECRET"));
            assert!(!d.generated_text);
        }
    }
}
