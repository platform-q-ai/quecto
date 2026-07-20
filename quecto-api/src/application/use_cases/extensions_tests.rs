use super::*;
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn list_forwards_get_extensions() {
    let gw = MockGateway::connected();
    list(&gw).await.unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::GetExtensions]
    ));
}

#[tokio::test]
async fn reload_forwards_reload_extensions() {
    let gw = MockGateway::connected();
    reload(&gw).await.unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::ReloadExtensions]
    ));
}

#[tokio::test]
async fn list_rejects_when_disconnected() {
    let err = list(&MockGateway::disconnected()).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
}

#[tokio::test]
async fn reload_rejects_when_disconnected() {
    let err = reload(&MockGateway::disconnected()).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
}
