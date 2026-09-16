//! [`EnvironmentMemberShutdown`] over the subagent teardown capability
//! (#1939): each member of an environment is a direct child of this session
//! (its row carries a launch generation), so its end is driven exactly like
//! a fleet teardown settles one child — claim, protocol shutdown over its
//! edge, the owned-handle fallback only for a handle this session owns,
//! and the row's exactly-once compensation — under the `EnvironmentKill`
//! cause, so the compensation runs no post-mortem and the environment's
//! own kill stays with the caller's claim. Mapping only: the ladder is the
//! application's.
use std::sync::Arc;

use crate::application::environments::ports::{
    EnvironmentMemberShutdown, MemberShutdownReport, MemberShutdownResult, PortFuture,
    SettledMember, UnsettledMember,
};
use crate::application::subagents::dto::FleetChildResult;
use crate::application::subagents::ports::{DelegatedAgentRegistry, ResolutionError};
use crate::application::subagents::use_cases::{ChildSettlement, SettleDelegatedChild};
use crate::domain::subagent_teardown::ShutdownReason;

pub struct DelegatedMemberShutdown {
    registry: Arc<dyn DelegatedAgentRegistry>,
    settle: Arc<SettleDelegatedChild>,
}

impl DelegatedMemberShutdown {
    pub fn new(
        registry: Arc<dyn DelegatedAgentRegistry>,
        settle: Arc<SettleDelegatedChild>,
    ) -> Self {
        Self { registry, settle }
    }

    async fn shutdown_member(&self, member: &str) -> Result<SettledMember, UnsettledMember> {
        let identity = match self.registry.resolve(member) {
            Ok(identity) => identity,
            // A row already gone or exited was ended by another path (its
            // membership left with it); nothing is left to ask.
            Err(ResolutionError::Unknown | ResolutionError::Exited) => {
                return Ok(SettledMember {
                    member: member.to_owned(),
                    result: MemberShutdownResult::AlreadyExited,
                });
            }
            Err(error @ (ResolutionError::Ambiguous | ResolutionError::NotDelegated)) => {
                return Err(UnsettledMember {
                    member: member.to_owned(),
                    detail: error.to_string(),
                });
            }
        };
        match self
            .settle
            .settle(
                identity,
                ShutdownReason::OperatorRequest,
                crate::application::subagents::ports::TerminationCause::EnvironmentKill,
            )
            .await
        {
            ChildSettlement::Done(settled) => Ok(SettledMember {
                member: member.to_owned(),
                result: match settled.result {
                    FleetChildResult::Graceful => MemberShutdownResult::Graceful,
                    FleetChildResult::Fallback => MemberShutdownResult::Fallback,
                    FleetChildResult::AlreadyExited => MemberShutdownResult::AlreadyExited,
                    FleetChildResult::Unobserved => MemberShutdownResult::Unobserved,
                    FleetChildResult::Joined => MemberShutdownResult::Joined,
                },
            }),
            ChildSettlement::Gone(_) => Ok(SettledMember {
                member: member.to_owned(),
                result: MemberShutdownResult::AlreadyExited,
            }),
            ChildSettlement::Unsettled(_, detail) => Err(UnsettledMember {
                member: member.to_owned(),
                detail,
            }),
        }
    }
}

impl EnvironmentMemberShutdown for DelegatedMemberShutdown {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        Box::pin(async move {
            let mut report = MemberShutdownReport::default();
            // Members are settled in membership order, one at a time: an
            // environment holds a handful of members, and the coordinator's
            // own harness settles its in-container descendants.
            for member in members {
                match self.shutdown_member(member).await {
                    Ok(settled) => report.settled.push(settled),
                    Err(unsettled) => report.unsettled.push(unsettled),
                }
            }
            report
        })
    }
}

/// The slots the agent-control tools read their composed use cases from
/// (#1936 review, #1939): the `agent_cmd kill` owner, the launch
/// lifecycle the spawn tool hands its reaper and monitor, and the
/// environment control `agent_cmd` serves (whose kill asks the composed
/// member shutdown directly). Built empty with the tools,
/// handed to composition inside the interface's wiring, filled once by
/// composition's graph. Cloning shares the slots.
#[derive(Clone, Default)]
pub struct TerminationSlots {
    pub kill: super::agent_cmd::KillToolSlot,
    pub lifecycle: super::subagent_teardown_wiring::SubagentLifecycleSlot,
    pub environments: super::agent_cmd_containers::EnvironmentControlSlot,
}

#[cfg(test)]
#[path = "environment_member_shutdown_tests.rs"]
mod tests;
