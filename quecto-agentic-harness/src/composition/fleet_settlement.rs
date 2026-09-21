//! The fleet teardown as the sessions capability's settlement port (D7
//! #1976, #1938): composition adapts the subagents use case the loop holds
//! late-bound to the `FleetSettlement` port a session transition orders, so
//! the two capabilities never name each other. A mapping only: each direct
//! child is told the operator asked, on the owner's authority (#2070 — a
//! transition settles its fleet only once its target is proven).
use std::future::Future;
use std::pin::Pin;

use crate::application::sessions::dto::{FleetSettled, FleetSettlementOutcome};
use crate::application::sessions::ports::FleetSettlement;
use crate::application::subagents::dto::{FleetTeardownError, TerminateAllDelegatedAgentsRequest};
use crate::application::subagents::use_cases::TerminateAllDelegatedAgents;
use crate::domain::subagent_teardown::ShutdownReason;

impl FleetSettlement for TerminateAllDelegatedAgents {
    fn settle_fleet(&self) -> Pin<Box<dyn Future<Output = FleetSettlementOutcome> + Send + '_>> {
        Box::pin(async move {
            match self
                .execute(TerminateAllDelegatedAgentsRequest {
                    reason: ShutdownReason::OperatorRequest,
                    authority: crate::application::subagents::dto::FleetTeardownAuthority::Owner,
                })
                .await
            {
                Err(FleetTeardownError::Interrupted) => FleetSettlementOutcome::Interrupted,
                Ok(outcome) if !outcome.is_settled() => {
                    FleetSettlementOutcome::Unsettled(outcome.unsettled)
                }
                Ok(outcome) => FleetSettlementOutcome::Settled(FleetSettled {
                    settled: outcome.settled.len(),
                    pruned: outcome.pruned.len(),
                    joined: outcome.joined,
                    removed: outcome.removed_count(),
                }),
            }
        })
    }
}
