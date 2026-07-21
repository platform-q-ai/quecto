use super::*;
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn rejects_empty_message() {
    let gw = MockGateway::connected();
    let err = execute(&gw, String::new()).await.unwrap_err();
    assert!(matches!(err, ApiError::InvalidRequest(_)));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn rejects_when_disconnected() {
    let gw = MockGateway::disconnected();
    let err = execute(&gw, "hi".into()).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
}

#[tokio::test]
async fn forwards_steer_command() {
    let gw = MockGateway::connected();
    execute(&gw, "focus".into()).await.unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::Steer { message }] if message == "focus"
    ));
}
