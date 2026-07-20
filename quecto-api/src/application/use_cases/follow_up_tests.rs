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
async fn forwards_follow_up_command() {
    let gw = MockGateway::connected();
    execute(&gw, "later".into()).await.unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::FollowUp { message }] if message == "later"
    ));
}
