// Handler-level and audit tests (split from router_tests.rs to respect file-size limits).
// Included via include! into the same #[cfg(test)] module, so it shares the test support above.
// ── Handler-level tests (direct invocation, no socket) ──────────────────────────

fn connected_gw() -> MockGateway {
    MockGateway {
        connected: true,
        ..MockGateway::default()
    }
}

fn failing_gw() -> MockGateway {
    MockGateway {
        connected: true,
        send_error: Some("boom".into()),
        ..MockGateway::default()
    }
}

fn state_for(gw: MockGateway) -> State<Arc<AppState<MockGateway>>> {
    State(Arc::new(AppState { gateway: gw }))
}

async fn status_of(resp: axum::response::Response) -> StatusCode {
    resp.status()
}

#[tokio::test]
async fn health_handler_reports_connected_and_disconnected() {
    let ok = health_handler(state_for(connected_gw()))
        .await
        .into_response();
    assert_eq!(ok.status(), StatusCode::OK);
    let down = health_handler(state_for(MockGateway::default()))
        .await
        .into_response();
    assert_eq!(down.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn prompt_handler_rejects_empty_message() {
    let resp = prompt_handler(
        state_for(connected_gw()),
        Json(PromptRequest {
            message: String::new(),
            streaming_behavior: None,
            wait_for_completion: true,
        }),
    )
    .await
    .into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn prompt_handler_forwards_and_maps_error() {
    let ok = prompt_handler(
        state_for(connected_gw()),
        Json(PromptRequest {
            message: "hi".into(),
            streaming_behavior: Some("steer".into()),
            wait_for_completion: true,
        }),
    )
    .await
    .into_response();
    assert_eq!(ok.status(), StatusCode::OK);

    let disconnected = prompt_handler(
        state_for(MockGateway::default()),
        Json(PromptRequest {
            message: "hi".into(),
            streaming_behavior: None,
            wait_for_completion: false,
        }),
    )
    .await
    .into_response();
    assert_eq!(disconnected.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn steer_follow_up_abort_handlers_forward() {
    assert_eq!(
        status_of(
            steer_handler(
                state_for(connected_gw()),
                Json(MessageRequest {
                    message: "go".into()
                })
            )
            .await
            .into_response()
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        status_of(
            follow_up_handler(
                state_for(connected_gw()),
                Json(MessageRequest {
                    message: "later".into()
                })
            )
            .await
            .into_response()
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        status_of(
            abort_handler(state_for(connected_gw()))
                .await
                .into_response()
        )
        .await,
        StatusCode::OK
    );
    // disconnected → 503
    assert_eq!(
        status_of(
            abort_handler(state_for(MockGateway::default()))
                .await
                .into_response()
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn steer_handler_maps_internal_error_to_500() {
    let resp = steer_handler(
        state_for(failing_gw()),
        Json(MessageRequest {
            message: "x".into(),
        }),
    )
    .await
    .into_response();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn set_model_handler_success_and_validation() {
    let ok = set_model_handler(
        state_for(connected_gw()),
        Json(SetModelRequest {
            model: Some("a/b".into()),
            provider: None,
            model_id: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(ok.status(), StatusCode::OK);

    let bad = set_model_handler(
        state_for(connected_gw()),
        Json(SetModelRequest {
            model: None,
            provider: None,
            model_id: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn set_effort_handler_success_and_validation() {
    let ok = set_effort_handler(
        state_for(connected_gw()),
        Json(SetEffortRequest {
            effort: "high".into(),
        }),
    )
    .await
    .into_response();
    assert_eq!(ok.status(), StatusCode::OK);

    let bad = set_effort_handler(
        state_for(connected_gw()),
        Json(SetEffortRequest {
            effort: "turbo".into(),
        }),
    )
    .await
    .into_response();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn clear_history_handler_connected_and_disconnected() {
    let ok = clear_history_handler(state_for(connected_gw()))
        .await
        .into_response();
    assert_eq!(ok.status(), StatusCode::OK);

    let down = clear_history_handler(state_for(MockGateway::default()))
        .await
        .into_response();
    assert_eq!(down.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn subagents_and_extensions_handlers_forward() {
    for resp in [
        subagents_handler(state_for(connected_gw()))
            .await
            .into_response(),
        extensions_handler(state_for(connected_gw()))
            .await
            .into_response(),
        extensions_reload_handler(state_for(connected_gw()))
            .await
            .into_response(),
        state_handler(state_for(connected_gw()))
            .await
            .into_response(),
    ] {
        assert_eq!(resp.status(), StatusCode::OK);
    }
    // disconnected
    assert_eq!(
        subagents_handler(state_for(MockGateway::default()))
            .await
            .into_response()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn messages_handlers_connected_and_disconnected() {
    let ok = messages_handler(
        state_for(connected_gw()),
        Query(MessagesQuery {
            before: Some("c".into()),
        }),
    )
    .await
    .into_response();
    assert_eq!(ok.status(), StatusCode::OK);

    let down = messages_handler(
        state_for(MockGateway::default()),
        Query(MessagesQuery { before: None }),
    )
    .await
    .into_response();
    assert_eq!(down.status(), StatusCode::SERVICE_UNAVAILABLE);

    let tail_ok = messages_tail_handler(state_for(connected_gw()), Query(TailQuery { n: Some(5) }))
        .await
        .into_response();
    assert_eq!(tail_ok.status(), StatusCode::OK);

    let tail_down = messages_tail_handler(
        state_for(MockGateway::default()),
        Query(TailQuery { n: None }),
    )
    .await
    .into_response();
    assert_eq!(tail_down.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn message_handler_rejects_when_disconnected() {
    let resp = message_handler(
        state_for(MockGateway::default()),
        axum::extract::Path("m1".into()),
        Query(MessageQuery {
            agent_id: None,
            offset: None,
            limit: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn stats_handler_connected_and_disconnected() {
    assert_eq!(
        stats_handler(state_for(connected_gw()))
            .await
            .into_response()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        stats_handler(state_for(MockGateway::default()))
            .await
            .into_response()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

// ── Error mapping ───────────────────────────────────────────────────────────────

#[test]
fn api_error_response_maps_every_variant() {
    use crate::domain::error::ApiError::*;
    assert_eq!(
        api_error_response(AgentNotConnected).0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(api_error_response(AgentBusy).0, StatusCode::CONFLICT);
    assert_eq!(
        api_error_response(Timeout(5)).0,
        StatusCode::GATEWAY_TIMEOUT
    );
    assert_eq!(
        api_error_response(InvalidRequest("x".into())).0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        api_error_response(Internal("x".into())).0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

// ── WS helper functions ─────────────────────────────────────────────────────────

#[test]
fn with_response_id_overrides_only_response_events() {
    let ev = with_response_id(
        AgentEvent::Response {
            id: Some("old".into()),
            command: "c".into(),
            success: true,
            data: None,
            error: None,
        },
        Some("new".into()),
    );
    match ev {
        AgentEvent::Response { id, .. } => assert_eq!(id.as_deref(), Some("new")),
        other => panic!("unexpected: {other:?}"),
    }
    // Non-Response passes through unchanged.
    assert!(matches!(
        with_response_id(AgentEvent::AgentStart, Some("x".into())),
        AgentEvent::AgentStart
    ));
}

#[test]
fn is_direct_ws_command_response_only_matches_get_message() {
    assert!(is_direct_ws_command_response(&AgentEvent::Response {
        id: None,
        command: "get_message".into(),
        success: true,
        data: None,
        error: None,
    }));
    assert!(!is_direct_ws_command_response(&AgentEvent::Response {
        id: None,
        command: "prompt".into(),
        success: true,
        data: None,
        error: None,
    }));
    assert!(!is_direct_ws_command_response(&AgentEvent::AgentStart));
}

#[test]
fn command_type_and_id_from_text() {
    let text = r#"{"type":"get_message","id":"abc"}"#;
    assert_eq!(command_type_from_text(text).as_deref(), Some("get_message"));
    assert_eq!(command_id_from_text(text).as_deref(), Some("abc"));
    assert_eq!(command_type_from_text("not json"), None);
    assert_eq!(command_id_from_text("{}"), None);
}

#[test]
fn ws_error_response_builds_failure_event() {
    let ev = ws_error_response(Some("i".into()), "get_message", "bad");
    match ev {
        AgentEvent::Response {
            id,
            command,
            success,
            error,
            ..
        } => {
            assert_eq!(id.as_deref(), Some("i"));
            assert_eq!(command, "get_message");
            assert!(!success);
            assert_eq!(error.as_deref(), Some("bad"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn default_wait_for_completion_is_true() {
    assert!(default_wait_for_completion());
}

// ── Audit events ────────────────────────────────────────────────────────────────

#[test]
fn sanitize_session_key_variants() {
    assert_eq!(sanitize_session_key("abc:def"), "abc_def");
    assert_eq!(sanitize_session_key("plain-key_1.2"), "plain-key_1.2");
    // dot-only / leading dot / empty → hex encoded with key_ prefix
    assert!(sanitize_session_key(".").starts_with("key_"));
    assert!(sanitize_session_key("").starts_with("key_"));
    assert!(sanitize_session_key("..").starts_with("key_"));
    // non-allowed characters → hex encoded
    assert!(sanitize_session_key("a/b").starts_with("key_"));
    assert!(sanitize_session_key("space here").starts_with("key_"));
}

#[test]
fn hex_encode_is_deterministic() {
    assert_eq!(hex_encode("AB"), "key_4142");
}

#[test]
fn audit_log_path_uses_env_overrides() {
    let _env = ENV_LOCK.blocking_lock();
    // SAFETY: single-threaded test; sets then reads process env.
    unsafe {
        std::env::set_var("QUECTO_BASE_DIR", "/tmp/quecto-test-base");
        std::env::set_var("QUECTO_SESSION_KEY", "sess:1");
    }
    let path = audit_log_path();
    assert!(
        path.to_string_lossy()
            .contains("/tmp/quecto-test-base/audit/")
    );
    assert!(path.to_string_lossy().ends_with("sess_1.jsonl"));
    // SAFETY: single-threaded test serialized by ENV_LOCK; sets/reads process env.
    unsafe {
        std::env::remove_var("QUECTO_BASE_DIR");
        std::env::remove_var("QUECTO_SESSION_KEY");
    }
}

#[test]
fn audit_log_path_falls_back_to_defaults_when_env_absent() {
    let _env = ENV_LOCK.blocking_lock();
    // Both vars are removed so the `unwrap_or_else` default branches are hit.
    // SAFETY: single-threaded test serialized by ENV_LOCK; sets/reads process env.
    unsafe {
        std::env::remove_var("QUECTO_BASE_DIR");
        std::env::remove_var("QUECTO_SESSION_KEY");
    }
    let path = audit_log_path();
    let shown = path.to_string_lossy();
    assert!(shown.starts_with("/home/appuser/.quecto/audit/"), "{shown}");
    assert!(shown.ends_with("default.jsonl"), "{shown}");
}

#[tokio::test]
async fn read_audit_events_missing_file_is_empty() {
    let _env = ENV_LOCK.lock().await;
    // SAFETY: single-threaded test serialized by ENV_LOCK; sets/reads process env.
    unsafe {
        std::env::set_var("QUECTO_BASE_DIR", "/nonexistent-quecto-dir-xyz");
        std::env::set_var("QUECTO_SESSION_KEY", "none");
    }
    let (events, next) = read_audit_events(0, 100).await.unwrap();
    assert!(events.is_empty());
    assert_eq!(next, 0);
    // SAFETY: single-threaded test serialized by ENV_LOCK; sets/reads process env.
    unsafe {
        std::env::remove_var("QUECTO_BASE_DIR");
        std::env::remove_var("QUECTO_SESSION_KEY");
    }
}

#[tokio::test]
async fn read_audit_events_paginates_existing_file() {
    let _env = ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let audit_dir = dir.path().join("audit");
    std::fs::create_dir_all(&audit_dir).unwrap();
    std::fs::write(
        audit_dir.join("s.jsonl"),
        "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\ninvalid json\n",
    )
    .unwrap();
    // SAFETY: single-threaded test serialized by ENV_LOCK; sets/reads process env.
    unsafe {
        std::env::set_var("QUECTO_BASE_DIR", dir.path());
        std::env::set_var("QUECTO_SESSION_KEY", "s");
    }
    let (events, next) = read_audit_events(1, 2).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["a"], 2);
    assert_eq!(next, 4);
    // SAFETY: single-threaded test serialized by ENV_LOCK; sets/reads process env.
    unsafe {
        std::env::remove_var("QUECTO_BASE_DIR");
        std::env::remove_var("QUECTO_SESSION_KEY");
    }
}

#[tokio::test]
async fn audit_events_handler_returns_ok_envelope() {
    let _env = ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let audit_dir = dir.path().join("audit");
    std::fs::create_dir_all(&audit_dir).unwrap();
    std::fs::write(audit_dir.join("h.jsonl"), "{\"x\":1}\n").unwrap();
    // SAFETY: single-threaded test serialized by ENV_LOCK; sets/reads process env.
    unsafe {
        std::env::set_var("QUECTO_BASE_DIR", dir.path());
        std::env::set_var("QUECTO_SESSION_KEY", "h");
    }
    let resp = audit_events_handler(Query(AuditEventsQuery {
        after: None,
        limit: None,
    }))
    .await
    .into_response();
    assert_eq!(resp.status(), StatusCode::OK);
    // SAFETY: single-threaded test serialized by ENV_LOCK; sets/reads process env.
    unsafe {
        std::env::remove_var("QUECTO_BASE_DIR");
        std::env::remove_var("QUECTO_SESSION_KEY");
    }
}

/// Serializes tests that mutate process-global environment variables.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn handlers_map_transport_failure_to_500() {
    // Every use-case-backed handler funnels gateway errors through
    // api_error_response; a failing gateway must surface as 500.
    assert_eq!(
        follow_up_handler(
            state_for(failing_gw()),
            Json(MessageRequest {
                message: "x".into()
            })
        )
        .await
        .into_response()
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        abort_handler(state_for(failing_gw()))
            .await
            .into_response()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        set_model_handler(
            state_for(failing_gw()),
            Json(SetModelRequest {
                model: Some("a/b".into()),
                provider: None,
                model_id: None,
            })
        )
        .await
        .into_response()
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        subagents_handler(state_for(failing_gw()))
            .await
            .into_response()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        extensions_handler(state_for(failing_gw()))
            .await
            .into_response()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        extensions_reload_handler(state_for(failing_gw()))
            .await
            .into_response()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        state_handler(state_for(failing_gw()))
            .await
            .into_response()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        stats_handler(state_for(failing_gw()))
            .await
            .into_response()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        messages_handler(
            state_for(failing_gw()),
            Query(MessagesQuery { before: None })
        )
        .await
        .into_response()
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        messages_tail_handler(state_for(failing_gw()), Query(TailQuery { n: None }))
            .await
            .into_response()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        message_handler(
            state_for(failing_gw()),
            axum::extract::Path("m".into()),
            Query(MessageQuery {
                agent_id: None,
                offset: None,
                limit: None
            }),
        )
        .await
        .into_response()
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn websocket_closes_when_subscribe_fails() {
    // A gateway whose subscribe() errors must cause the socket to close
    // immediately rather than hang.
    #[derive(Clone)]
    struct NoSubscribeGateway;
    impl AgentGateway for NoSubscribeGateway {
        fn send(
            &self,
            _cmd: AgentCommand,
        ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
            Box::pin(async { Err(ApiError::AgentNotConnected) })
        }
        fn enqueue(
            &self,
            cmd: AgentCommand,
        ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
            self.send(cmd)
        }
        fn subscribe(
            &self,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            Box<dyn crate::application::ports::agent_gateway::EventSubscriber>,
                            ApiError,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async { Err(ApiError::AgentNotConnected) })
        }
        fn is_connected(&self) -> bool {
            true
        }
    }

    let app = build_router(NoSubscribeGateway);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("connect");
    // The server should close the stream promptly.
    let next = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
        .await
        .expect("close arrives within 2s");
    assert!(
        matches!(
            next,
            None | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | Some(Err(_))
        ),
        "expected close/end, got {next:?}"
    );
}
