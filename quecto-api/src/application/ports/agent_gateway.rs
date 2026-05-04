/// Port: agent gateway — the application's contract for communicating
/// with a quecto agent process.
///
/// The infrastructure layer provides the UDS implementation. Tests use
/// a mock. The application layer never imports UDS types directly.
use std::future::Future;
use std::pin::Pin;

use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// A command to send to the agent.
#[derive(Debug, Clone)]
pub enum AgentCommand {
    Prompt {
        message: String,
        streaming_behavior: Option<String>,
    },
    Abort,
    GetState,
    GetMessages,
    GetMessagesTail {
        count: usize,
    },
    GetSessionStats,
    SetModel {
        model: Option<String>,
        provider: Option<String>,
        model_id: Option<String>,
    },
    ClearHistory,
}

/// Subscriber handle — receives broadcast events from the agent.
pub trait EventSubscriber: Send + Sync {
    /// Receive the next event. Returns None when the agent disconnects.
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = Option<AgentEvent>> + Send + '_>>;
}

/// Gateway to a quecto agent.
///
/// Implementations are expected to be cheaply cloneable (Arc-based).
pub trait AgentGateway: Send + Sync + 'static {
    /// Send a command to the agent and get the response event.
    fn send(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>>;

    /// Send a command to the agent without waiting for command completion.
    fn enqueue(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>>;

    /// Subscribe to the agent's broadcast event stream.
    /// Each subscriber gets its own copy of every event.
    #[allow(clippy::type_complexity)]
    fn subscribe(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn EventSubscriber>, ApiError>> + Send + '_>>;

    /// Check if the agent is connected.
    fn is_connected(&self) -> bool;
}
