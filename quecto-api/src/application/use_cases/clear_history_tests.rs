use super::*;
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn forwards_clear_history_when_connected() {
    let gw = MockGateway::connected();
    execute(&gw).await.unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::ClearHistory]
    ));
}

#[tokio::test]
async fn rejects_when_disconnected() {
    let gw = MockGateway::disconnected();
    let err = execute(&gw).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn propagates_transport_error() {
    let gw = MockGateway::failing(ApiError::Timeout(60));
    assert!(matches!(
        execute(&gw).await.unwrap_err(),
        ApiError::Timeout(60)
    ));
}
