use super::*;
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn forwards_get_state_when_connected() {
    let gw = MockGateway::connected();
    execute(&gw).await.unwrap();
    assert!(matches!(gw.commands().as_slice(), [AgentCommand::GetState]));
}

#[tokio::test]
async fn rejects_when_disconnected() {
    let gw = MockGateway::disconnected();
    let err = execute(&gw).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
    assert!(gw.commands().is_empty());
}
