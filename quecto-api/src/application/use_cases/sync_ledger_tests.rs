use super::*;
use crate::application::ports::agent_gateway::AgentCommand;
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn forwards_sync_request_to_gateway() {
    let gateway = MockGateway::connected();
    let event = execute(
        &gateway,
        SyncInput {
            epoch: 7,
            since_rev: 3,
            agent_id: Some("worker".into()),
        },
    )
    .await
    .expect("sync succeeds");

    assert!(
        matches!(event, crate::domain::event::AgentEvent::Response { command, .. } if command == "sync")
    );
    assert!(matches!(
        &gateway.commands()[..],
        [AgentCommand::Sync { epoch: 7, since_rev: 3, agent_id }] if agent_id.as_deref() == Some("worker")
    ));
}
