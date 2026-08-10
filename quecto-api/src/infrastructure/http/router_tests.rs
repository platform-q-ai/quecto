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
            let command = match self.commands.lock().unwrap().last() {
                Some(AgentCommand::Sync { .. }) => "sync",
                _ => "get_message",
            };
            Ok(AgentEvent::Response {
                id: Some("req".into()),
                command: command.into(),
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
async fn websocket_typed_prompt_returns_direct_response() {
    let gateway = MockGateway {
        connected: true,
        ..MockGateway::default()
    };
    let response = ws_response_for(
        gateway.clone(),
        serde_json::json!({"type":"prompt","message":"hello"}),
    )
    .await;

    assert!(
        gateway
            .commands
            .lock()
            .unwrap()
            .iter()
            .any(|cmd| matches!(cmd, AgentCommand::Prompt { message, .. } if message == "hello"),)
    );
    assert_eq!(response["type"], "response");
    assert_eq!(response["command"], "get_message");
    assert_eq!(response["success"], true);
}

#[tokio::test]
async fn websocket_sync_forwards_cursor_and_agent_id() {
    let gateway = MockGateway {
        connected: true,
        ..MockGateway::default()
    };
    let response = ws_response_for(
        gateway.clone(),
        serde_json::json!({
            "type":"sync",
            "id":"sync-client-1",
            "epoch":7,
            "sinceRev":3,
            "agent_id":"worker"
        }),
    )
    .await;

    assert_eq!(response["id"], "sync-client-1");
    assert_eq!(response["command"], "sync");
    let commands = gateway.commands.lock().unwrap();
    assert!(matches!(
        &commands[..],
        [AgentCommand::Sync { epoch: 7, since_rev: 3, agent_id }] if agent_id.as_deref() == Some("worker")
    ));
}

#[tokio::test]
async fn websocket_sync_rejects_invalid_shape_without_gateway_send() {
    let gateway = MockGateway {
        connected: true,
        ..MockGateway::default()
    };
    let response = ws_response_for(
        gateway.clone(),
        serde_json::json!({"type":"sync","id":"bad-sync","epoch":7}),
    )
    .await;

    assert_eq!(response["type"], "response");
    assert_eq!(response["id"], "bad-sync");
    assert_eq!(response["command"], "sync");
    assert_eq!(response["success"], false);
    assert!(gateway.commands.lock().unwrap().is_empty());
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
