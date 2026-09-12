use super::super::ack_fakes::RecordingWriter;
use super::*;
use crate::application::subagents::dto::{PreparedShutdown, ShutdownToken};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration, RoutingDepth, ShutdownReason, TerminationRouteError,
};

fn prepared() -> PreparedShutdown {
    PreparedShutdown {
        token: ShutdownToken::for_presenter_tests(),
        joined: false,
        reason: ShutdownReason::ParentShutdown,
    }
}

#[test]
fn shutdown_ack_is_correlated_and_names_the_reason_only() {
    let ack = shutdown_ack(Some("c-7"), &prepared());
    assert_eq!(ack.id.as_deref(), Some("c-7"));
    assert_eq!(ack.command, "shutdown");
    assert!(ack.success);
    assert_eq!(
        ack.data,
        Some(TeardownResponseData::ShuttingDown {
            reason: "parent_shutdown".into()
        })
    );
    // The opaque token never appears on the wire.
    assert!(!ack.to_line().contains("token"));
}

#[test]
fn rejections_carry_the_use_case_vocabulary() {
    let shutdown = shutdown_rejection(None, &HarnessShutdownError::AlreadyTerminated);
    assert!(!shutdown.success);
    assert_eq!(
        shutdown.error.as_deref(),
        Some("harness already terminated")
    );
    let termination = termination_rejection(
        Some("t"),
        &TerminateDelegatedAgentError::Rejected(TerminationRouteError::TargetIsSelf),
    );
    assert_eq!(termination.command, "terminate_delegated_agent");
    assert_eq!(
        termination.error.as_deref(),
        Some("termination rejected: target is the receiving harness; use shutdown")
    );
    let edge = edge_rejection(Some("e"), SHUTDOWN_COMMAND, "unauthorized connection");
    assert_eq!(edge.id.as_deref(), Some("e"));
    assert_eq!(edge.error.as_deref(), Some("unauthorized connection"));
}

#[test]
fn termination_responses_distinguish_shutdown_from_forward() {
    let child = DelegatedAgentIdentity::new("B", LaunchGeneration::new(1));
    let requested = termination_response(
        Some("t"),
        &TerminationRouted::ShutdownRequested {
            child: child.clone(),
        },
    );
    assert_eq!(
        requested.data,
        Some(TeardownResponseData::ShutdownRequested {
            child_uuid: "B".into()
        })
    );
    let forwarded = termination_response(
        None,
        &TerminationRouted::Forwarded {
            via: DelegatedAgentIdentity::new(AgentUuid::new("A"), LaunchGeneration::new(1)),
            remaining_depth: RoutingDepth::new(2).unwrap(),
        },
    );
    assert_eq!(
        forwarded.data,
        Some(TeardownResponseData::Forwarded {
            via_uuid: "A".into(),
            remaining_depth: 2
        })
    );
}

#[tokio::test]
async fn deliver_writes_one_newline_terminated_frame_and_surfaces_flush_failure() {
    let writer = RecordingWriter::default();
    deliver(&writer, &shutdown_ack(Some("c"), &prepared()))
        .await
        .unwrap();
    let frames = writer.frames();
    assert_eq!(frames.len(), 1);
    assert!(frames[0].ends_with('\n'));
    assert_eq!(frames[0].matches('\n').count(), 1);
    let failing = RecordingWriter::failing();
    assert_eq!(
        deliver(&failing, &shutdown_ack(Some("c"), &prepared())).await,
        Err(AckWriteError("broken pipe".into()))
    );
    assert!(failing.frames().is_empty());
    assert_eq!(AckWriteError("x".into()).to_string(), "ack write failed: x");
}
