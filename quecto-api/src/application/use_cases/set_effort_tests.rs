use super::*;
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn forwards_valid_effort_normalized() {
    let gw = MockGateway::connected();
    execute(&gw, "  HIGH ".into()).await.unwrap();
    assert!(matches!(
        gw.commands().as_slice(),
        [AgentCommand::SetEffort { effort }] if effort == "high"
    ));
}

#[tokio::test]
async fn accepts_every_documented_level() {
    for level in ["none", "low", "medium", "high", "xhigh", "max"] {
        let gw = MockGateway::connected();
        execute(&gw, level.into()).await.unwrap();
        assert_eq!(gw.commands().len(), 1, "level={level}");
    }
}

#[tokio::test]
async fn rejects_unknown_effort() {
    let gw = MockGateway::connected();
    let err = execute(&gw, "turbo".into()).await.unwrap_err();
    assert!(matches!(err, ApiError::InvalidRequest(_)));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn rejects_empty_effort() {
    let gw = MockGateway::connected();
    let err = execute(&gw, "   ".into()).await.unwrap_err();
    assert!(matches!(err, ApiError::InvalidRequest(_)));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn rejects_when_disconnected() {
    let gw = MockGateway::disconnected();
    let err = execute(&gw, "high".into()).await.unwrap_err();
    assert!(matches!(err, ApiError::AgentNotConnected));
    assert!(gw.commands().is_empty());
}

#[tokio::test]
async fn propagates_transport_error() {
    let gw = MockGateway::failing(ApiError::Timeout(30));
    assert!(matches!(
        execute(&gw, "low".into()).await.unwrap_err(),
        ApiError::Timeout(30)
    ));
}
