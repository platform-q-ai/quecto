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
