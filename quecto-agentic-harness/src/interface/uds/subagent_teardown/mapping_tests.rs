use super::*;
use crate::domain::subagent_teardown::RoutingDepth;

fn shutdown(reason: &str) -> SubagentTeardownCommand {
    SubagentTeardownCommand::Shutdown {
        id: Some("c".into()),
        reason: reason.into(),
    }
}

fn terminate(uuid: &str, generation: u64, depth: u32) -> SubagentTeardownCommand {
    SubagentTeardownCommand::TerminateDelegatedAgent {
        id: None,
        target_uuid: uuid.into(),
        target_generation: generation,
        remaining_depth: depth,
    }
}

#[test]
fn every_canonical_reason_maps_to_a_protocol_triggered_prepare() {
    for reason in ShutdownReason::ALL {
        assert_eq!(
            map_command(&shutdown(reason.as_str())),
            Ok(TeardownRequest::Shutdown(PrepareShutdownRequest {
                reason,
                trigger: ShutdownTrigger::ProtocolCommand,
            }))
        );
    }
}

#[test]
fn unknown_reasons_are_rejected() {
    assert_eq!(
        map_command(&shutdown("kill")),
        Err(MappingError::UnknownReason("kill".into()))
    );
    assert_eq!(
        MappingError::UnknownReason("kill".into()).to_string(),
        "unknown shutdown reason \"kill\""
    );
}

#[test]
fn terminate_maps_identity_and_bounded_depth() {
    assert_eq!(
        map_command(&terminate("B", 4, 3)),
        Ok(TeardownRequest::TerminateDelegatedAgent(
            TerminateDelegatedAgentRequest {
                target: DelegatedAgentIdentity::new("B", LaunchGeneration::new(4)),
                remaining_depth: RoutingDepth::new(3).unwrap(),
            }
        ))
    );
}

#[test]
fn zero_and_excess_depth_and_empty_uuid_are_rejected() {
    assert_eq!(
        map_command(&terminate("B", 1, 0)),
        Err(MappingError::InvalidDepth(
            "remaining_depth must be at least 1".into()
        ))
    );
    assert_eq!(
        map_command(&terminate("B", 1, RoutingDepth::MAX_HOPS + 1)),
        Err(MappingError::InvalidDepth(
            "remaining_depth 33 exceeds maximum 32".into()
        ))
    );
    assert_eq!(
        map_command(&terminate("   ", 1, 1)),
        Err(MappingError::EmptyTargetUuid)
    );
    assert_eq!(
        MappingError::EmptyTargetUuid.to_string(),
        "target_uuid must not be empty"
    );
    assert_eq!(MappingError::InvalidDepth("d".into()).to_string(), "d");
}
