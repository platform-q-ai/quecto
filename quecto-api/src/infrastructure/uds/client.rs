// UDS client — infrastructure adapter implementing the AgentGateway port.
//
// Connects to a quecto agent's Unix domain socket, sends JSON messages,
// and distributes incoming events to subscribers via broadcast channels.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use std::collections::HashMap;
use tokio::io::BufReader;
use tokio::net::UnixStream;

use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

struct PendingResponse {
    command: String,
    tx: oneshot::Sender<AgentEvent>,
}

use quecto_line_io::{FrameError, WireMode, read_frame_or_legacy_line_into, write_message};

use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway, EventSubscriber};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;
use uuid::Uuid;

/// Maximum line size (8 MiB, matching quecto's protocol limit).
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
    /// One-shot waiters for correlated command responses.
    pending_responses: Arc<Mutex<HashMap<String, PendingResponse>>>,
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

        // Event broadcast and correlated direct-command waiters.
        let (event_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let pending_responses = Arc::new(Mutex::new(HashMap::<String, PendingResponse>::new()));
        let connected = Arc::new(AtomicBool::new(true));

        // Event reader task
        let reader_tx = event_tx.clone();
        let reader_pending = pending_responses.clone();
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
                                let mut delivered_to_pending = false;
                                if let AgentEvent::Response { id: Some(id), .. } = &event {
                                    if let Some(pending) = reader_pending.lock().await.remove(id) {
                                        delivered_to_pending = true;
                                        let _ = pending.tx.send(event.clone());
                                    }
                                }
                                if let AgentEvent::Response { command, .. } = &event {
                                    if command == "agent_error" {
                                        let mut pending = reader_pending.lock().await;
                                        if let Some(id) = pending.iter().find_map(|(id, p)| {
                                            (p.command == "prompt").then(|| id.clone())
                                        }) {
                                            if let Some(pending) = pending.remove(&id) {
                                                delivered_to_pending = true;
                                                let _ = pending.tx.send(event.clone());
                                            }
                                        }
                                    }
                                }
                                // Correlated direct-command responses are returned by
                                // `send`; fan-out subscribers should not receive a
                                // second copy. Other events are broadcast normally.
                                if !delivered_to_pending {
                                    let _ = reader_tx.send(event);
                                }
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
            pending_responses,
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
        AgentCommand::Steer { message } => {
            serde_json::json!({"type": "steer", "id": id, "message": message})
        }
        AgentCommand::FollowUp { message } => {
            serde_json::json!({"type": "follow_up", "id": id, "message": message})
        }
        AgentCommand::GetState => serde_json::json!({"type": "get_state", "id": id}),
        AgentCommand::GetMessages { before } => {
            let mut v = serde_json::json!({"type": "get_messages", "id": id});
            if let Some(before) = before {
                v["before"] = serde_json::Value::String(before);
            }
            v
        }
        AgentCommand::GetMessagesTail { count } => {
            serde_json::json!({"type": "get_messages_tail", "id": id, "count": count})
        }
        AgentCommand::Sync {
            epoch,
            since_rev,
            agent_id,
        } => {
            let mut v = serde_json::json!({"type": "sync", "id": id, "epoch": epoch, "sinceRev": since_rev});
            if let Some(agent_id) = agent_id {
                v["agent_id"] = serde_json::Value::String(agent_id);
            }
            v
        }
        AgentCommand::GetMessage {
            message_id,
            agent_id,
            tool_call_id,
            offset,
            limit,
        } => {
            let mut v = serde_json::json!({
                "type": "get_message", "id": id, "messageId": message_id
            });
            if let Some(agent_id) = agent_id {
                v["agent_id"] = serde_json::Value::String(agent_id);
            }
            if let Some(tool_call_id) = tool_call_id {
                v["toolCallId"] = serde_json::Value::String(tool_call_id);
            }
            if let Some(offset) = offset {
                v["offset"] = serde_json::json!(offset);
            }
            if let Some(limit) = limit {
                v["limit"] = serde_json::json!(limit);
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
        AgentCommand::SetEffort { effort } => {
            serde_json::json!({"type": "set_effort", "id": id, "effort": effort})
        }
        AgentCommand::ClearHistory => serde_json::json!({"type": "clear_history", "id": id}),
        AgentCommand::GetSubagents => serde_json::json!({"type": "get_subagents", "id": id}),
        AgentCommand::GetToolCatalogue => {
            serde_json::json!({"type": "get_tool_catalogue", "id": id})
        }
        AgentCommand::SetToolPolicy {
            mutations,
            mode,
            operation,
            unlisted_scope,
        } => {
            let mut obj = serde_json::json!({"type": "set_tool_policy", "id": id, "mutations": mutations, "mode": mode});
            if operation
                == crate::application::ports::agent_gateway::ToolPolicyOperationPayload::Replace
            {
                obj["operation"] = serde_json::to_value(operation).unwrap();
            }
            if let Some(scope) = unlisted_scope {
                obj["unlistedScope"] = serde_json::to_value(scope).unwrap();
            }
            obj
        }
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

            let (response_tx, response_rx) = oneshot::channel();
            let command_name = json_value["type"].as_str().unwrap_or_default().to_owned();
            this.pending_responses.lock().await.insert(
                id.clone(),
                PendingResponse {
                    command: command_name,
                    tx: response_tx,
                },
            );

            if let Err(err) = this.send_raw(line).await {
                this.pending_responses.lock().await.remove(&id);
                return Err(err);
            }

            // Wait for the correlated response event without depending on the
            // lossy broadcast receiver used for fan-out subscribers.
            let timeout = tokio::time::Duration::from_secs(120);
            match tokio::time::timeout(timeout, response_rx).await {
                Ok(Ok(event)) => Ok(event),
                Ok(Err(_)) => Err(ApiError::AgentNotConnected),
                Err(_) => {
                    this.pending_responses.lock().await.remove(&id);
                    Err(ApiError::Timeout(timeout.as_secs()))
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
#[path = "client_tests.rs"]
mod tests;
