use super::*;
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;
use futures::{SinkExt, StreamExt};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct MockGateway {
    commands: Arc<Mutex<Vec<AgentCommand>>>,
    connected: bool,
    send_error: Option<String>,
}

struct PendingSubscriber;

#[derive(Clone)]
struct BroadcastingGateway {
    events: tokio::sync::broadcast::Sender<AgentEvent>,
}

struct BroadcastSubscriber(tokio::sync::broadcast::Receiver<AgentEvent>);

impl crate::application::ports::agent_gateway::EventSubscriber for BroadcastSubscriber {
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = Option<AgentEvent>> + Send + '_>> {
        Box::pin(async { self.0.recv().await.ok() })
    }
}

impl AgentGateway for BroadcastingGateway {
    fn send(
        &self,
        _cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
        let events = self.events.clone();
        Box::pin(async move {
            let event = AgentEvent::Response {
                id: Some("internal-id".into()),
                command: "get_message".into(),
                success: true,
                data: Some(serde_json::json!({"content": "page"})),
                error: None,
            };
            let _ = events.send(event.clone());
            Ok(event)
        })
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
        let receiver = self.events.subscribe();
        Box::pin(async move { Ok(Box::new(BroadcastSubscriber(receiver)) as _) })
    }

    fn is_connected(&self) -> bool {
        true
    }
}

impl crate::application::ports::agent_gateway::EventSubscriber for PendingSubscriber {
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = Option<AgentEvent>> + Send + '_>> {
        Box::pin(std::future::pending())
    }
}

impl AgentGateway for MockGateway {
    fn send(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
        self.commands.lock().unwrap().push(cmd);
        let send_error = self.send_error.clone();
        Box::pin(async move {
            if let Some(message) = send_error {
                return Err(ApiError::Internal(message));
            }
            Ok(AgentEvent::Response {
                id: Some("req".into()),
                command: "get_message".into(),
                success: true,
                data: Some(serde_json::json!({"ok": true})),
                error: None,
            })
        })
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
        Box::pin(async { Ok(Box::new(PendingSubscriber) as _) })
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

async fn ws_response_for(gateway: MockGateway, text: serde_json::Value) -> serde_json::Value {
    let app = build_router(gateway);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("websocket connects");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        text.to_string().into(),
    ))
    .await
    .expect("send websocket command");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
        .await
        .expect("websocket response arrives")
        .expect("websocket stream item")
        .expect("websocket message succeeds");
    let tokio_tungstenite::tungstenite::Message::Text(text) = msg else {
        panic!("expected text response, got {msg:?}");
    };
    serde_json::from_str(&text).expect("websocket response is json")
}

#[tokio::test]
async fn websocket_malformed_command_returns_structured_error() {
    let response = ws_response_for(
        MockGateway {
            connected: true,
            ..MockGateway::default()
        },
        serde_json::json!({"type":"get_message","id":"bad-command","offset":0}),
    )
    .await;

    assert_eq!(response["type"], "response");
    assert_eq!(response["id"], "bad-command");
    assert_eq!(response["command"], "get_message");
    assert_eq!(response["success"], false);
    assert!(
        response["error"]
            .as_str()
            .is_some_and(|message| message.contains("invalid request")),
        "parse failure should be reported to the WebSocket client: {response}"
    );
}

#[tokio::test]
async fn websocket_broadcasting_gateway_delivers_one_direct_response() {
    let (events, _) = tokio::sync::broadcast::channel(8);
    let app = build_router(BroadcastingGateway { events });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("websocket connects");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({
            "type":"get_message", "id":"client-id", "messageId":"m1"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let first = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
        .await
        .expect("direct response arrives")
        .unwrap()
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&first.into_text().unwrap()).unwrap();
    assert_eq!(value["id"], "client-id");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), ws.next())
            .await
            .is_err(),
        "the gateway broadcast must not produce a duplicate WebSocket frame"
    );
}

#[tokio::test]
async fn websocket_success_preserves_client_correlation_id() {
    let response = ws_response_for(
        MockGateway {
            connected: true,
            ..MockGateway::default()
        },
        serde_json::json!({
            "type":"get_message",
            "id":"client-page-1",
            "messageId":"m1",
            "offset":0,
            "limit":1024
        }),
    )
    .await;

    assert_eq!(response["id"], "client-page-1");
    assert_eq!(response["command"], "get_message");
    assert_eq!(response["success"], true);
}

#[tokio::test]
async fn websocket_typed_prompt_reaches_legacy_prompt_handler() {
    let gateway = MockGateway {
        connected: true,
        ..MockGateway::default()
    };
    let app = build_router(gateway.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("websocket connects");
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({"type":"prompt","message":"hello"})
            .to_string()
            .into(),
    ))
    .await
    .expect("typed prompt sends");

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if gateway.commands.lock().unwrap().iter().any(
                |cmd| matches!(cmd, AgentCommand::Prompt { message, .. } if message == "hello"),
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("typed prompt reaches gateway");
}

#[tokio::test]
async fn websocket_gateway_send_failure_returns_structured_error() {
    let gateway = MockGateway {
        connected: true,
        send_error: Some("uds write failed".into()),
        ..MockGateway::default()
    };
    let response = ws_response_for(
        gateway.clone(),
        serde_json::json!({"type":"get_message","id":"send-failed","messageId":"m1"}),
    )
    .await;

    assert_eq!(gateway.commands.lock().unwrap().len(), 1);
    assert_eq!(response["type"], "response");
    assert_eq!(response["id"], "send-failed");
    assert_eq!(response["command"], "get_message");
    assert_eq!(response["success"], false);
    assert!(
        response["error"]
            .as_str()
            .is_some_and(|message| message.contains("uds write failed")),
        "gateway send failure should be reported to the WebSocket client: {response}"
    );
}

#[tokio::test]
async fn message_handler_forwards_range_query_to_gateway() {
    let gateway = MockGateway {
        connected: true,
        ..MockGateway::default()
    };
    let state = Arc::new(AppState {
        gateway: gateway.clone(),
    });

    let response = message_handler(
        State(state),
        axum::extract::Path("msg-1".to_string()),
        Query(MessageQuery {
            agent_id: Some("worker".into()),
            offset: Some(4096),
            limit: Some(8192),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let commands = gateway.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    match &commands[0] {
        AgentCommand::GetMessage {
            message_id,
            agent_id,
            tool_call_id: None,
            offset,
            limit,
        } => {
            assert_eq!(message_id, "msg-1");
            assert_eq!(agent_id.as_deref(), Some("worker"));
            assert_eq!(*offset, Some(4096));
            assert_eq!(*limit, Some(8192));
        }
        other => panic!("expected ranged get_message command, got {other:?}"),
    }
}

include!("router_handler_tests.rs");
