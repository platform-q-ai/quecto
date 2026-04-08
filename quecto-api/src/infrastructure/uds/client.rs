// UDS client — infrastructure adapter implementing the AgentGateway port.
//
// Connects to a quecto agent's Unix domain socket, sends JSON-lines commands,
// and distributes incoming events to subscribers via broadcast channels.

use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::future::Future;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc};

use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway, EventSubscriber};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Maximum line size (1 MiB, matching quecto's protocol limit).
const MAX_LINE_BYTES: usize = 1_048_576;

/// Broadcast channel capacity — how many events can be buffered for slow subscribers.
const BROADCAST_CAPACITY: usize = 512;

/// UDS-based implementation of [AgentGateway].
///
/// Cheaply cloneable (all fields are Arc-wrapped). Call [UdsGateway::connect]
/// to establish the connection, then pass clones to axum handlers.
#[derive(Clone)]
pub struct UdsGateway {
    /// Channel to send serialised JSON-lines commands to the writer task.
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
    /// - A reader that parses incoming JSON-lines events and broadcasts them.
    pub async fn connect(socket_path: &Path) -> Result<Self, ApiError> {
        let stream = UnixStream::connect(socket_path)
            .await
            .map_err(|e| ApiError::Internal(format!("UDS connect failed: {e}")))?;

        let (read_half, mut write_half) = tokio::io::split(stream);

        // Command writer task
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<String>(64);
        tokio::spawn(async move {
            while let Some(line) = cmd_rx.recv().await {
                if write_half.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if write_half.flush().await.is_err() {
                    break;
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
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        if line.len() > MAX_LINE_BYTES {
                            tracing::warn!("dropping oversized line ({} bytes)", line.len());
                            continue;
                        }
                        let trimmed = line.trim();
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
                    Err(e) => {
                        tracing::error!("UDS read error: {e}");
                        break;
                    }
                }
                // Reclaim memory if a large line inflated the buffer.
                if line.capacity() > 64 * 1024 {
                    line.shrink_to(8 * 1024);
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

    /// Send a raw JSON-lines command string.
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

            let id = uuid::Uuid::new_v4().to_string();

            let json_value = match cmd {
                AgentCommand::Prompt { message, streaming_behavior } => {
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
                AgentCommand::Abort => serde_json::json!({
                    "type": "abort",
                    "id": id,
                }),
                AgentCommand::GetState => serde_json::json!({
                    "type": "get_state",
                    "id": id,
                }),
                AgentCommand::GetMessages => serde_json::json!({
                    "type": "get_messages",
                    "id": id,
                }),
                AgentCommand::GetMessagesTail { count } => serde_json::json!({
                    "type": "get_messages_tail",
                    "id": id,
                    "count": count,
                }),
                AgentCommand::GetSessionStats => serde_json::json!({
                    "type": "get_session_stats",
                    "id": id,
                }),
                AgentCommand::SetModel { model, provider, model_id } => {
                    let mut obj = serde_json::json!({
                        "type": "set_model",
                        "id": id,
                    });
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
                AgentCommand::ClearHistory => serde_json::json!({
                    "type": "clear_history",
                    "id": id,
                }),
            };

            let mut line = serde_json::to_string(&json_value)
                .map_err(|e| ApiError::Internal(format!("serialization error: {e}")))?;
            line.push('\n');

            // Subscribe BEFORE sending so we don't miss the response.
            let mut rx = this.event_tx.subscribe();
            this.send_raw(line).await?;

            // Wait for the correlated response event.
            let timeout = tokio::time::Duration::from_secs(120);
            let deadline = tokio::time::Instant::now() + timeout;

            loop {
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Ok(event)) => {
                        // Check if this is our correlated response.
                        if let AgentEvent::Response { id: ref resp_id, .. } = event {
                            if resp_id.as_deref() == Some(id.as_str()) {
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

    fn subscribe(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn EventSubscriber>, ApiError>> + Send + '_>> {
        let rx = self.event_tx.subscribe();
        Box::pin(async move {
            Ok(Box::new(BroadcastSubscriber { rx }) as Box<dyn EventSubscriber>)
        })
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}
