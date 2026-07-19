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
    /// Interrupt after the current tool, then deliver this message.
    Steer {
        message: String,
    },
    /// Deliver this message when the agent finishes the current run.
    FollowUp {
        message: String,
    },
    Abort,
    GetState,
    /// #1061: history is paged — the agent returns the newest bounded page plus
    /// `before`/`hasMoreBefore` metadata. Pass `before` (a stable message id
    /// from a prior page) to fetch the adjacent older page.
    GetMessages {
        before: Option<String>,
    },
    GetMessagesTail {
        count: usize,
    },
    /// #1060: resolve a single message by its stable id (the on-demand lookup
    /// path for refs carried on end-of-turn events). `agent_id` forwards the
    /// lookup to a spawned child.
    GetMessage {
        message_id: String,
        agent_id: Option<String>,
        tool_call_id: Option<String>,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    GetSessionStats,
    SetModel {
        model: Option<String>,
        provider: Option<String>,
        model_id: Option<String>,
    },
    /// Set the reasoning effort for subsequent turns. Validated against the
    /// active model's provider vocabulary by the agent.
    SetEffort {
        effort: String,
    },
    /// Clear conversation history in-place without restarting the agent.
    ClearHistory,
    /// #524: list spawned subagents and their live status.
    GetSubagents,
    /// List registered extensions.
    GetExtensions,
    /// Re-scan extension directories and reload script extensions.
    ReloadExtensions,
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
    #[expect(
        clippy::type_complexity,
        reason = "async trait object return keeps port object-safe"
    )]
    fn subscribe(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn EventSubscriber>, ApiError>> + Send + '_>>;

    /// Check if the agent is connected.
    fn is_connected(&self) -> bool;
}
