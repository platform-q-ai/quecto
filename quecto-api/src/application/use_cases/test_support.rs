//! Shared in-crate test doubles for application use-case unit tests.
//!
//! Kept behind `#[cfg(test)]` so it never ships in the production binary. The
//! mock records the commands it receives so use-case tests can assert on the
//! exact `AgentCommand` produced, independent of any UDS transport. It can also
//! be primed to fail, so error-propagation paths are covered without a socket.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway, EventSubscriber};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Records every command and returns a canned outcome.
#[derive(Clone)]
pub struct MockGateway {
    connected: bool,
    /// When set, `send`/`enqueue` fail with this error instead of succeeding.
    fail_with: Option<Arc<ApiError>>,
    sent: Arc<Mutex<Vec<AgentCommand>>>,
    enqueued: Arc<Mutex<Vec<AgentCommand>>>,
    subscribe_ok: bool,
}

impl MockGateway {
    pub fn connected() -> Self {
        Self {
            connected: true,
            fail_with: None,
            sent: Arc::new(Mutex::new(Vec::new())),
            enqueued: Arc::new(Mutex::new(Vec::new())),
            subscribe_ok: true,
        }
    }

    pub fn disconnected() -> Self {
        Self {
            connected: false,
            ..Self::connected()
        }
    }

    /// A connected gateway whose transport calls fail with `err`.
    pub fn failing(err: ApiError) -> Self {
        Self {
            fail_with: Some(Arc::new(err)),
            ..Self::connected()
        }
    }

    /// A connected gateway whose `subscribe` fails.
    pub fn subscribe_failing() -> Self {
        Self {
            subscribe_ok: false,
            ..Self::connected()
        }
    }

    /// Commands received via `send`.
    pub fn commands(&self) -> Vec<AgentCommand> {
        self.sent.lock().unwrap().clone()
    }

    /// Commands received via `enqueue`.
    pub fn enqueued(&self) -> Vec<AgentCommand> {
        self.enqueued.lock().unwrap().clone()
    }

    fn outcome(&self, command: &str) -> Result<AgentEvent, ApiError> {
        if let Some(err) = &self.fail_with {
            return Err(clone_error(err));
        }
        Ok(AgentEvent::Response {
            id: Some("mock".into()),
            command: command.into(),
            success: true,
            data: Some(serde_json::json!({"ok": true})),
            error: None,
        })
    }
}

/// `ApiError` is not `Clone`; reproduce the variant for a fresh error value.
fn clone_error(err: &ApiError) -> ApiError {
    match err {
        ApiError::AgentNotConnected => ApiError::AgentNotConnected,
        ApiError::AgentBusy => ApiError::AgentBusy,
        ApiError::Timeout(s) => ApiError::Timeout(*s),
        ApiError::InvalidRequest(m) => ApiError::InvalidRequest(m.clone()),
        ApiError::Internal(m) => ApiError::Internal(m.clone()),
    }
}

fn command_name(cmd: &AgentCommand) -> &'static str {
    match cmd {
        AgentCommand::Prompt { .. } => "prompt",
        AgentCommand::Steer { .. } => "steer",
        AgentCommand::FollowUp { .. } => "follow_up",
        AgentCommand::Abort => "abort",
        AgentCommand::GetState => "get_state",
        AgentCommand::GetMessages { .. } => "get_messages",
        AgentCommand::GetMessagesTail { .. } => "get_messages_tail",
        AgentCommand::GetMessage { .. } => "get_message",
        AgentCommand::GetSessionStats => "get_session_stats",
        AgentCommand::SetModel { .. } => "set_model",
        AgentCommand::ClearHistory => "clear_history",
        AgentCommand::GetSubagents => "get_subagents",
        AgentCommand::GetExtensions => "get_extensions",
        AgentCommand::ReloadExtensions => "reload_extensions",
    }
}

impl AgentGateway for MockGateway {
    fn send(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
        let name = command_name(&cmd);
        self.sent.lock().unwrap().push(cmd);
        let outcome = self.outcome(name);
        Box::pin(async move { outcome })
    }

    fn enqueue(
        &self,
        cmd: AgentCommand,
    ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
        let name = command_name(&cmd);
        self.enqueued.lock().unwrap().push(cmd);
        let outcome = self.outcome(name);
        Box::pin(async move { outcome })
    }

    fn subscribe(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn EventSubscriber>, ApiError>> + Send + '_>> {
        let ok = self.subscribe_ok;
        Box::pin(async move {
            if ok {
                Ok(Box::new(NullSubscriber) as Box<dyn EventSubscriber>)
            } else {
                Err(ApiError::AgentNotConnected)
            }
        })
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

/// A subscriber that never yields (the stream is closed immediately).
struct NullSubscriber;

impl EventSubscriber for NullSubscriber {
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = Option<AgentEvent>> + Send + '_>> {
        Box::pin(async { None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_sent_and_enqueued_commands() {
        let gw = MockGateway::connected();
        gw.send(AgentCommand::Abort).await.unwrap();
        gw.enqueue(AgentCommand::GetState).await.unwrap();
        assert_eq!(gw.commands().len(), 1);
        assert_eq!(gw.enqueued().len(), 1);
    }

    #[tokio::test]
    async fn failing_gateway_propagates_error() {
        let gw = MockGateway::failing(ApiError::Timeout(3));
        let err = gw.send(AgentCommand::Abort).await.unwrap_err();
        assert!(matches!(err, ApiError::Timeout(3)));
    }

    #[tokio::test]
    async fn subscribe_can_be_forced_to_fail() {
        assert!(MockGateway::subscribe_failing().subscribe().await.is_err());
        assert!(MockGateway::connected().subscribe().await.is_ok());
    }

    #[test]
    fn clone_error_covers_all_variants() {
        for err in [
            ApiError::AgentNotConnected,
            ApiError::AgentBusy,
            ApiError::Timeout(1),
            ApiError::InvalidRequest("x".into()),
            ApiError::Internal("y".into()),
        ] {
            let cloned = clone_error(&err);
            assert_eq!(cloned.to_string(), err.to_string());
        }
    }
}
