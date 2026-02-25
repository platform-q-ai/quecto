//! Async channel bus for nonblocking coordinator communication.
//!
//! The coordinator runs as a `tokio::spawn` background task. The main agent's
//! `coding_job` tool sends commands via the inbound channel and receives
//! responses via a per-command `oneshot` response channel. This keeps the
//! agent loop fully non-blocking.

use tokio::sync::{mpsc, oneshot};

/// A command sent from the agent's coding_job tool to the coordinator.
#[derive(Debug)]
pub struct CoordinatorCommand {
    /// The raw JSON action string from the tool invocation.
    pub action_json: String,
    /// One-shot channel to send the response back to the caller.
    pub reply_tx: oneshot::Sender<CoordinatorResponse>,
}

/// A response from the coordinator back to the tool caller.
#[derive(Debug, Clone)]
pub struct CoordinatorResponse {
    /// `true` if the command succeeded.
    pub ok: bool,
    /// The JSON-serialized response body.
    pub body: String,
    /// Error message on failure.
    pub error: Option<String>,
}

/// Async channel bus for coordinator commands.
///
/// Follows the same take-once receiver pattern as `MessageBus`:
/// - Senders are cloneable and distributed to tool instances.
/// - The single receiver is taken by the background coordinator task.
#[derive(Debug)]
pub struct CoordinatorBus {
    cmd_tx: mpsc::Sender<CoordinatorCommand>,
    cmd_rx: Option<mpsc::Receiver<CoordinatorCommand>>,
    buffer_size: usize,
}

impl CoordinatorBus {
    /// Create a new coordinator bus with the given channel buffer size.
    pub fn new(buffer: usize) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(buffer);
        Self {
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            buffer_size: buffer,
        }
    }

    /// Get a cloneable sender for dispatching commands to the coordinator.
    pub fn command_sender(&self) -> mpsc::Sender<CoordinatorCommand> {
        self.cmd_tx.clone()
    }

    /// Take the command receiver (used by the background coordinator task).
    /// Can only be called once; subsequent calls return `None`.
    pub fn take_command_receiver(&mut self) -> Option<mpsc::Receiver<CoordinatorCommand>> {
        self.cmd_rx.take()
    }

    /// The configured buffer size.
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Whether the receiver has already been taken.
    pub fn receiver_taken(&self) -> bool {
        self.cmd_rx.is_none()
    }

    /// Check if the command channel is closed (all senders dropped).
    pub fn is_closed(&self) -> bool {
        self.cmd_tx.is_closed()
    }
}

/// Handle held by the background coordinator task to process commands.
///
/// Wraps the receiver side and provides a simple `recv()` + `respond()` API.
#[derive(Debug)]
pub struct CoordinatorHandle {
    rx: mpsc::Receiver<CoordinatorCommand>,
}

impl CoordinatorHandle {
    /// Create a handle from a taken receiver.
    pub fn new(rx: mpsc::Receiver<CoordinatorCommand>) -> Self {
        Self { rx }
    }

    /// Receive the next command. Returns `None` when all senders are dropped
    /// (i.e., shutdown).
    pub async fn recv(&mut self) -> Option<CoordinatorCommand> {
        self.rx.recv().await
    }
}

/// Dispatch scope policy: determines whether the coordinator runs as a
/// background task or synchronously via `spawn_blocking`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchMode {
    /// Gateway mode: coordinator is a `tokio::spawn` background task,
    /// commands flow through async channels.
    Background,
    /// CLI agent mode: coordinator is constructed per-session,
    /// commands dispatch synchronously via `spawn_blocking`.
    Synchronous,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_command_roundtrip() {
        let mut bus = CoordinatorBus::new(16);
        let sender = bus.command_sender();
        let mut rx = bus.take_command_receiver().unwrap();

        let (reply_tx, reply_rx) = oneshot::channel();
        sender
            .send(CoordinatorCommand {
                action_json: r#"{"action":"status","job_id":"j1"}"#.to_string(),
                reply_tx,
            })
            .await
            .unwrap();

        let cmd = rx.recv().await.unwrap();
        assert!(cmd.action_json.contains("status"));
        cmd.reply_tx
            .send(CoordinatorResponse {
                ok: true,
                body: r#"{"state":"running"}"#.to_string(),
                error: None,
            })
            .unwrap();

        let resp = reply_rx.await.unwrap();
        assert!(resp.ok);
        assert!(resp.body.contains("running"));
    }

    #[test]
    fn test_take_receiver_only_once() {
        let mut bus = CoordinatorBus::new(16);
        assert!(bus.take_command_receiver().is_some());
        assert!(bus.take_command_receiver().is_none());
        assert!(bus.receiver_taken());
    }

    #[tokio::test]
    async fn test_channel_backpressure() {
        let mut bus = CoordinatorBus::new(2);
        let sender = bus.command_sender();
        let _rx = bus.take_command_receiver().unwrap();

        // Fill the buffer
        for i in 0..2 {
            let (reply_tx, _reply_rx) = oneshot::channel();
            sender
                .send(CoordinatorCommand {
                    action_json: format!(r#"{{"action":"status","i":{i}}}"#),
                    reply_tx,
                })
                .await
                .unwrap();
        }

        // Third send should not complete immediately (channel full)
        let (reply_tx, _reply_rx) = oneshot::channel();
        let send_result = sender.try_send(CoordinatorCommand {
            action_json: r#"{"action":"run"}"#.to_string(),
            reply_tx,
        });
        assert!(send_result.is_err(), "channel should be full");
    }

    #[tokio::test]
    async fn test_receiver_returns_none_on_sender_drop() {
        let mut bus = CoordinatorBus::new(16);
        let sender = bus.command_sender();
        let mut rx = bus.take_command_receiver().unwrap();

        drop(sender);
        // Drop the internal sender too by dropping the bus
        drop(bus);

        let result = rx.recv().await;
        assert!(
            result.is_none(),
            "recv should return None after all senders dropped"
        );
    }

    #[tokio::test]
    async fn test_multiple_commands_buffered() {
        let mut bus = CoordinatorBus::new(16);
        let sender = bus.command_sender();
        let mut rx = bus.take_command_receiver().unwrap();

        // Send 5 commands
        let mut reply_rxs = Vec::new();
        for i in 0..5 {
            let (reply_tx, reply_rx) = oneshot::channel();
            sender
                .send(CoordinatorCommand {
                    action_json: format!(r#"{{"action":"status","i":{i}}}"#),
                    reply_tx,
                })
                .await
                .unwrap();
            reply_rxs.push(reply_rx);
        }

        // Process all 5 from receiver side
        for i in 0..5 {
            let cmd = rx.recv().await.unwrap();
            cmd.reply_tx
                .send(CoordinatorResponse {
                    ok: true,
                    body: format!("resp_{i}"),
                    error: None,
                })
                .unwrap();
        }

        // Verify all responses
        for (i, reply_rx) in reply_rxs.into_iter().enumerate() {
            let resp = reply_rx.await.unwrap();
            assert!(resp.ok);
            assert_eq!(resp.body, format!("resp_{i}"));
        }
    }

    #[test]
    fn test_buffer_size() {
        let bus = CoordinatorBus::new(42);
        assert_eq!(bus.buffer_size(), 42);
    }

    #[test]
    fn test_dispatch_mode_values() {
        assert_ne!(DispatchMode::Background, DispatchMode::Synchronous);
    }

    #[tokio::test]
    async fn test_coordinator_handle_recv() {
        let mut bus = CoordinatorBus::new(16);
        let sender = bus.command_sender();
        let rx = bus.take_command_receiver().unwrap();
        let mut handle = CoordinatorHandle::new(rx);

        let (reply_tx, _reply_rx) = oneshot::channel();
        sender
            .send(CoordinatorCommand {
                action_json: r#"{"action":"run"}"#.to_string(),
                reply_tx,
            })
            .await
            .unwrap();

        let cmd = handle.recv().await;
        assert!(cmd.is_some());
        assert!(cmd.unwrap().action_json.contains("run"));
    }

    #[tokio::test]
    async fn test_coordinator_handle_returns_none_on_shutdown() {
        let mut bus = CoordinatorBus::new(16);
        let sender = bus.command_sender();
        let rx = bus.take_command_receiver().unwrap();
        let mut handle = CoordinatorHandle::new(rx);

        drop(sender);
        drop(bus);

        let cmd = handle.recv().await;
        assert!(cmd.is_none(), "handle should return None after shutdown");
    }
}
