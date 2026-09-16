//! The departing session's delegated children (#1938, D7 #1976): the
//! narrow collaborator every session transition runs before its final
//! save and before anything is replaced. Settle first: the fleet teardown
//! claims each direct child, asks it to shut down over its edge, concludes
//! it and compensates it, under a bound; a child that does not settle
//! refuses the transition explicitly — the roster is left as it was — so
//! ownership of a live child is never silently dropped. Replace last: once
//! the fleet settled every row left is a record only; a live delegated
//! row here would be a child the teardown did not own, so it is refused
//! rather than dropped.
//!
//! Shared by the fresh-session and the resume transactions (D8 #1977);
//! the two remain distinct transactions.
use std::sync::Arc;

use crate::application::sessions::dto::{
    FleetSettlementOutcome, SessionTransition, SessionTransitionRefused,
};
use crate::application::sessions::ports::{DelegatedChildrenRoster, FleetSettlement};

pub struct DepartingChildren {
    /// `None` when the loop tracks no roster: nothing to settle or replace.
    roster: Option<Arc<dyn DelegatedChildrenRoster>>,
}

impl DepartingChildren {
    pub fn new(roster: Option<Arc<dyn DelegatedChildrenRoster>>) -> Self {
        Self { roster }
    }

    /// Settle the current session's direct children through `fleet`;
    /// returns how many rows left the roster. Without a fleet, live
    /// delegated rows refuse the transition and a roster of records only
    /// passes.
    pub async fn settle(
        &self,
        fleet: Option<&dyn FleetSettlement>,
        transition: SessionTransition,
    ) -> Result<usize, SessionTransitionRefused> {
        let Some(fleet) = fleet else {
            let live = self.live_delegated_rows();
            return if live == 0 {
                Ok(0)
            } else {
                Err(SessionTransitionRefused::NoFleetTeardown(live))
            };
        };
        match fleet.settle_fleet().await {
            FleetSettlementOutcome::Interrupted => Err(SessionTransitionRefused::Interrupted),
            FleetSettlementOutcome::Unsettled(children) => {
                tracing::warn!(
                    transition = transition.as_str(),
                    unsettled = children.len(),
                    "session switch refused: departing children did not settle"
                );
                Err(SessionTransitionRefused::Unsettled(children))
            }
            FleetSettlementOutcome::Settled(settled) => {
                tracing::info!(
                    transition = transition.as_str(),
                    settled = settled.settled,
                    pruned = settled.pruned,
                    joined = settled.joined,
                    "session switch: departing children settled before the roster is replaced"
                );
                Ok(settled.removed)
            }
        }
    }

    /// Replace the operational roster once the fleet has settled; returns
    /// how many records were dropped.
    pub fn reset_roster(
        &self,
        transition: SessionTransition,
    ) -> Result<usize, SessionTransitionRefused> {
        let Some(roster) = &self.roster else {
            return Ok(0);
        };
        let live = roster.live_delegated_rows();
        if live > 0 {
            return Err(SessionTransitionRefused::LiveRowsRemain(live));
        }
        let dropped = roster.clear_roster();
        if dropped > 0 {
            tracing::info!(
                transition = transition.as_str(),
                dropped,
                "session switch: roster records replaced"
            );
        }
        Ok(dropped)
    }

    /// Note what a resumed session's persisted rows are: history only
    /// (#1937). A launcher-created child is lifetime-scoped to the harness
    /// that launched it, so no persisted record can describe a live child
    /// of *this* harness. Nothing is probed, compared, monitored or
    /// readopted; the master re-spawns what it needs. Logged only when the
    /// loop tracks a roster the rows would otherwise have joined.
    pub fn note_persisted_rows_are_history(&self, persisted_rows: usize) {
        if self.roster.is_some() && persisted_rows > 0 {
            tracing::info!(
                ignored_rows = persisted_rows,
                "session restore: persisted subagent rows are history only; no child readopted"
            );
        }
    }

    fn live_delegated_rows(&self) -> usize {
        self.roster
            .as_ref()
            .map_or(0, |roster| roster.live_delegated_rows())
    }
}

impl std::fmt::Debug for DepartingChildren {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DepartingChildren")
            .field("tracks_roster", &self.roster.is_some())
            .finish()
    }
}

#[cfg(test)]
#[path = "departing_children_tests.rs"]
mod tests;
