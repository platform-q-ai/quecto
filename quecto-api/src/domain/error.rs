/// Domain errors for quecto-api.
///
/// These are transport-agnostic — they describe what went wrong without
/// leaking HTTP status codes, WebSocket close codes, or UDS I/O details.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("agent not connected")]
    AgentNotConnected,

    #[error("agent is busy; provide streamingBehavior (steer or followUp)")]
    AgentBusy,

    #[error("request timed out after {0}s")]
    Timeout(u64),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("internal error: {0}")]
    Internal(String),
}
