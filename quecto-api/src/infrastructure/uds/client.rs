// UDS client — infrastructure adapter implementing the AgentGateway port.
//
// Connects to a quecto agent's Unix domain socket, sends JSON messages,
// and distributes incoming events to subscribers via broadcast channels.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc};

use quecto_line_io::{FrameError, WireMode, read_frame_or_legacy_line_into, write_message};

use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway, EventSubscriber};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;
use uuid::Uuid;

/// Maximum line size (1 MiB, matching quecto's protocol limit).
const MAX_LINE_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES;

/// Broadcast channel capacity — how many events can be buffered for slow subscribers.
const BROADCAST_CAPACITY: usize = 512;

/// UDS-based implementation of [AgentGateway].
///
/// Cheaply cloneable (all fields are Arc-wrapped). Call [UdsGateway::connect]
/// to establish the connection, then pass clones to axum handlers.
#[derive(Clone)]
pub struct UdsGateway {
    /// Channel to send serialised JSON commands to the writer task.
    cmd_tx: mpsc::Sender<String>,
    /// Broadcast sender — every incoming event is sent here.
    event_tx: broadcast::Sender<AgentEvent>,
    /// Whether the background reader is still alive.
    connected: Arc<AtomicBool>,
}

impl std::fmt::Debug for UdsGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdsGateway")
            .field("connected", &self.connected.load(Ordering::Relaxed))
            .finish()
    }
}

impl UdsGateway {
    /// Connect to a quecto agent at the given socket path.
    ///
    /// Spawns two background tasks:
    /// - A writer that drains the command channel and writes to the socket.
    /// - A reader that parses incoming JSON events and broadcasts them.
    pub async fn connect(socket_path: &Path) -> Result<Self, ApiError> {
        let stream = UnixStream::connect(socket_path)
            .await
            .map_err(|e| ApiError::Internal(format!("UDS connect failed: {e}")))?;

        let (read_half, mut write_half) = tokio::io::split(stream);

        // Command writer task. The gateway attaches to an already-running,
        // separately-versioned agent by socket path — there is no stderr
        // announcement to negotiate against — so during the ADR-0008 NDJSON
        // deprecation window it writes commands as legacy NDJSON lines. That is
        // the one framing BOTH agent generations accept: a pre-#1059 agent
        // reads NDJSON natively, and a current agent's reader sniffs each
        // message and replies in the same (legacy) framing. Writing frames here
        // would make a pre-#1059 agent's newline reader hang forever on the
        // first (newline-less) frame — the silent hang ADR-0008 forbids. (When
        // part 3 closes the window this path needs an explicit version
        // handshake; out of scope for part 1.) The reader below stays
        // dual-mode so a current agent's framed replies still parse.
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<String>(64);
        tokio::spawn(async move {
            while let Some(line) = cmd_rx.recv().await {
                let payload = line.strip_suffix('\n').unwrap_or(&line).as_bytes();
                match write_message(
                    &mut write_half,
                    payload,
                    WireMode::LegacyLine,
                    MAX_LINE_BYTES,
                )
                .await
                {
                    Ok(()) => {}
                    // An over-cap command is refused with nothing on the wire.
                    // Drop just that one message and keep the shared writer
                    // alive — a per-message validation refusal must not tear
                    // down the single gateway connection for every client.
                    Err(FrameError::Oversized { .. }) => {
                        tracing::warn!("dropping oversized outbound command");
                        continue;
                    }
                    // A real transport error is fatal to this connection.
                    Err(e) => {
                        tracing::error!("UDS write error: {e}");
                        break;
                    }
                }
            }
        });

        // Event broadcast
        let (event_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let connected = Arc::new(AtomicBool::new(true));

        // Event reader task
        let reader_tx = event_tx.clone();
        let reader_connected = connected.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(read_half);
            // Reused across iterations so a streaming turn does not allocate a
            // fresh payload buffer per event (#1059 review).
            let mut bytes: Vec<u8> = Vec::new();
            loop {
                // Deprecation-window reader (#1059): each incoming message is
                // sniffed as a length-prefixed frame or a legacy NDJSON line,
                // so this client interoperates with both agent generations.
                match read_frame_or_legacy_line_into(&mut reader, &mut bytes, MAX_LINE_BYTES).await
                {
                    Ok(None) => break,
                    Ok(Some(_wire_mode)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        let trimmed = text.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<AgentEvent>(trimmed) {
                            Ok(event) => {
                                // Ignore send errors — means no subscribers currently.
                                let _ = reader_tx.send(event);
                            }
                            Err(e) => {
                                let preview_len = trimmed.len().min(200);
                                tracing::warn!(
                                    "failed to parse agent event: {e} (preview: {}...)",
                                    &trimmed[..preview_len]
                                );
                            }
                        }
                    }
                    Err(e @ FrameError::Oversized { .. }) => {
                        // Rejected cleanly; the stream stays framed.
                        tracing::warn!("dropping oversized message from agent: {e}");
                        continue;
                    }
                    Err(e) => {
                        // I/O error or an explicit protocol version mismatch
                        // (never a hang or a silent misparse).
                        tracing::error!("UDS read error: {e}");
                        break;
                    }
                }
            }
            reader_connected.store(false, Ordering::Relaxed);
            tracing::info!("UDS reader exited — agent disconnected");
        });

        Ok(Self {
            cmd_tx,
            event_tx,
            connected,
        })
    }

    /// Send a raw JSON command payload.
    async fn send_raw(&self, json: String) -> Result<(), ApiError> {
        self.cmd_tx
            .send(json)
            .await
            .map_err(|_| ApiError::AgentNotConnected)
    }
}

/// Subscriber that receives broadcast events.
struct BroadcastSubscriber {
    rx: broadcast::Receiver<AgentEvent>,
}

impl EventSubscriber for BroadcastSubscriber {
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = Option<AgentEvent>> + Send + '_>> {
        Box::pin(async {
            loop {
                match self.rx.recv().await {
                    Ok(event) => return Some(event),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("subscriber lagged, dropped {n} events");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
    }
}

fn command_to_json(cmd: AgentCommand, id: &str) -> serde_json::Value {
    match cmd {
        AgentCommand::Prompt {
            message,
            streaming_behavior,
        } => {
            let mut obj = serde_json::json!({
                "type": "prompt",
                "id": id,
                "message": message,
            });
            if let Some(sb) = streaming_behavior {
                obj["streamingBehavior"] = serde_json::Value::String(sb);
            }
            obj
        }
        AgentCommand::Abort => serde_json::json!({"type": "abort", "id": id}),
        AgentCommand::GetState => serde_json::json!({"type": "get_state", "id": id}),
        AgentCommand::GetMessages => serde_json::json!({"type": "get_messages", "id": id}),
        AgentCommand::GetMessagesTail { count } => {
            serde_json::json!({"type": "get_messages_tail", "id": id, "count": count})
        }
        AgentCommand::GetMessage {
            message_id,
            agent_id,
        } => {
            let mut v = serde_json::json!({
                "type": "get_message", "id": id, "messageId": message_id
            });
            if let Some(agent_id) = agent_id {
                v["agent_id"] = serde_json::Value::String(agent_id);
            }
            v
        }
        AgentCommand::GetSessionStats => {
            serde_json::json!({"type": "get_session_stats", "id": id})
        }
        AgentCommand::SetModel {
            model,
            provider,
            model_id,
        } => {
            let mut obj = serde_json::json!({"type": "set_model", "id": id});
            if let Some(m) = model {
                obj["model"] = serde_json::Value::String(m);
            }
            if let Some(p) = provider {
                obj["provider"] = serde_json::Value::String(p);
            }
            if let Some(mi) = model_id {
                obj["modelId"] = serde_json::Value::String(mi);
            }
            obj
        }
        AgentCommand::ClearHistory => serde_json::json!({"type": "clear_history", "id": id}),
    }
}

impl AgentGateway for UdsGateway {
    fn send(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
        let this = self.clone();
        Box::pin(async move {
            if !this.is_connected() {
                return Err(ApiError::AgentNotConnected);
            }

            let id = Uuid::new_v4().to_string();
            let json_value = command_to_json(cmd, &id);

            let mut line = serde_json::to_string(&json_value)
                .map_err(|e| ApiError::Internal(format!("serialization error: {e}")))?;
            line.push('\n');

            // Subscribe BEFORE sending so we don't miss the response.
            let mut rx = this.event_tx.subscribe();
            this.send_raw(line).await?;

            // Wait for the correlated response event.
            let timeout = tokio::time::Duration::from_secs(120);
            let deadline = tokio::time::Instant::now() + timeout;

            // Determine the command name for fallback matching (agent_error
            // responses may not carry the correlation ID).
            let command_name = match &json_value["type"] {
                serde_json::Value::String(s) => s.clone(),
                _ => String::new(),
            };

            loop {
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Ok(event)) => {
                        if let AgentEvent::Response {
                            id: ref resp_id,
                            ref command,
                            ..
                        } = event
                        {
                            // Match by correlation ID if present.
                            if resp_id.as_deref() == Some(id.as_str()) {
                                return Ok(event);
                            }
                            // Fallback: match agent_error responses for our
                            // command type (they may omit the correlation ID).
                            if command == "agent_error" && command_name == "prompt" {
                                return Ok(event);
                            }
                        }
                        // Not our response — keep waiting.
                    }
                    Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                        tracing::warn!("send() subscriber lagged, dropped {n} events");
                    }
                    Ok(Err(broadcast::error::RecvError::Closed)) => {
                        return Err(ApiError::AgentNotConnected);
                    }
                    Err(_) => return Err(ApiError::Timeout(timeout.as_secs())),
                }
            }
        })
    }

    fn enqueue(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
        let this = self.clone();
        Box::pin(async move {
            if !this.is_connected() {
                return Err(ApiError::AgentNotConnected);
            }

            let id = Uuid::new_v4().to_string();
            let json_value = command_to_json(cmd, &id);
            let command_name = json_value["type"].as_str().unwrap_or("command").to_string();
            let mut line = serde_json::to_string(&json_value)
                .map_err(|e| ApiError::Internal(format!("serialization error: {e}")))?;
            line.push('\n');
            this.send_raw(line).await?;

            Ok(AgentEvent::Response {
                id: Some(id),
                command: command_name,
                success: true,
                data: Some(serde_json::json!({"accepted": true})),
                error: None,
            })
        })
    }

    fn subscribe(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn EventSubscriber>, ApiError>> + Send + '_>> {
        let rx = self.event_tx.subscribe();
        Box::pin(
            async move { Ok(Box::new(BroadcastSubscriber { rx }) as Box<dyn EventSubscriber>) },
        )
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────
//
// #1003: the event reader now reads lines through the shared
// `quecto_line_io::read_bounded_line` helper instead of `read_line` +
// post-hoc length check, so an oversized, unterminated line from the agent
// is capped *while being read* rather than fully buffered and only checked
// afterward. These tests drive `UdsGateway::connect` over a real socket and
// assert on the one observable effect: which events actually reach a
// subscriber.
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn oversized_event_line_is_dropped_but_later_valid_events_still_arrive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("agent.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind test socket");

        // Gate the server writes on the subscription existing: broadcast
        // sends before `subscribe()` are dropped (see the reader loop in
        // `connect`), so writing immediately after accept races the
        // subscription and flakes.
        let (subscribed_tx, subscribed_rx) = tokio::sync::oneshot::channel::<()>();
        let accept_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            subscribed_rx.await.expect("subscribed signal");
            // One giant unterminated-then-terminated line, well over
            // MAX_LINE_BYTES, followed by a normal, valid event.
            let oversized = format!(
                "{{\"type\":\"token\",\"token\":\"{}\"}}\n",
                "x".repeat(MAX_LINE_BYTES + 65_536)
            );
            stream
                .write_all(oversized.as_bytes())
                .await
                .expect("write oversized line");
            stream
                .write_all(b"{\"type\":\"token\",\"token\":\"hi\"}\n")
                .await
                .expect("write valid line");
        });

        let gateway = UdsGateway::connect(&socket_path)
            .await
            .expect("connect to agent socket");
        let mut sub = gateway.subscribe().await.expect("subscribe");
        subscribed_tx.send(()).expect("signal subscribed");

        let event = tokio::time::timeout(std::time::Duration::from_secs(3), sub.recv())
            .await
            .expect("subscriber should still receive an event after an oversized line within 3s")
            .expect("event present");

        match event {
            AgentEvent::Token { token } => assert_eq!(
                token, "hi",
                "the oversized line must not be delivered as a parsed event, only the valid one that follows it"
            ),
            other => panic!("expected a Token event, got: {other:?}"),
        }

        accept_task.await.expect("accept task completed");
    }

    #[test]
    fn max_line_bytes_matches_documented_protocol_limit() {
        assert_eq!(MAX_LINE_BYTES, 1_048_576);
    }

    // ── #1060 lockstep: refs preserved through the API event model ──────────

    #[test]
    fn get_message_command_serializes_to_wire() {
        let v = command_to_json(
            AgentCommand::GetMessage {
                message_id: "m1".into(),
                agent_id: None,
            },
            "req1",
        );
        assert_eq!(v["type"], "get_message");
        assert_eq!(v["messageId"], "m1");
        assert_eq!(v["id"], "req1");
        assert!(v.get("agent_id").is_none());

        let child = command_to_json(
            AgentCommand::GetMessage {
                message_id: "m2".into(),
                agent_id: Some("worker".into()),
            },
            "req2",
        );
        assert_eq!(child["agent_id"], "worker");
    }

    #[test]
    fn agent_end_preserves_message_refs_round_trip() {
        // The harness emits agent_end refs-based (empty messages) after #1060.
        let wire = r#"{"type":"agent_end","messages":[],"messageRefs":["a","b"]}"#;
        let ev: AgentEvent = serde_json::from_str(wire).expect("parse agent_end");
        match &ev {
            AgentEvent::AgentEnd { message_refs, .. } => {
                assert_eq!(message_refs, &vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected AgentEnd, got {other:?}"),
        }
        // Re-serialized to a WS client, the refs must survive (not be dropped).
        let out = serde_json::to_value(&ev).unwrap();
        assert_eq!(out["messageRefs"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn subagent_messages_appended_is_modeled_not_unknown() {
        let wire = r#"{"type":"subagent_messages_appended","agent_id":"worker","messages":[],"messageRefs":["x"]}"#;
        let ev: AgentEvent = serde_json::from_str(wire).expect("parse");
        match &ev {
            AgentEvent::SubagentMessagesAppended {
                agent_id,
                message_refs,
                ..
            } => {
                assert_eq!(agent_id, "worker");
                assert_eq!(message_refs, &vec!["x".to_string()]);
            }
            other => panic!("expected SubagentMessagesAppended, not Unknown; got {other:?}"),
        }
        let out = serde_json::to_value(&ev).unwrap();
        assert_eq!(out["type"], "subagent_messages_appended");
        assert_eq!(out["messageRefs"], serde_json::json!(["x"]));
    }
}
