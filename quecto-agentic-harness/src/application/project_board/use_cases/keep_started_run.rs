//! Keeps a started run (S4d, #2479): one look at a time, as
//! [`keeper`](crate::application::project_board::keeper) says. The run's
//! keeper calls [`KeepStartedRun::look`] every
//! [`KEEPER_PERIOD`](crate::application::project_board::keeper::KEEPER_PERIOD)
//! until a look ends the keeping or its session ends; what a look did is
//! the keeper's to record.
//!
//! Blocking: the store's and the swarm's reads, and a renewal's or a
//! fold-back's write, hold the calling thread.
use std::sync::Arc;

use super::board_change::checked_writer;
use super::{ChangeBoardTask, FoldBackRun, board_view};
use crate::application::project_board::dto::{BoardOp, FoldedBack, ProjectBoardError};
use crate::application::project_board::keeper::{
    KeeperEnd, KeptRun, Look, RENEW_BELOW_SECONDS, Renewal, STALL_LIMIT, SwarmSight, Watch,
    sight_of,
};
use crate::application::project_board::ports::{
    BoardClock, BoardStoreError, BoardTextCells, FoldBackRecorder, ProjectBoardStore, StartedSwarm,
};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;

pub struct KeepStartedRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    swarm: Arc<dyn StartedSwarm>,
    change: ChangeBoardTask,
    fold_back: FoldBackRun,
}

/// The kept run as the board holds it.
enum OnBoard {
    /// Going: the seconds its task's claim has left, when it has one.
    Open { claim_left: Option<i64> },
    /// Ended already.
    Ended,
    /// The task, or the run, is not on the board.
    Gone,
}

impl KeepStartedRun {
    /// A keeper writing to `store` (never cancelled: a renewal or a
    /// fold-back outlives the call that started the run), reading the
    /// run's swarm through `swarm`.
    ///
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        cells: Arc<dyn BoardTextCells>,
        swarm: Arc<dyn StartedSwarm>,
    ) -> Result<Self, ProjectBoardError> {
        checked_writer(&*store)?;
        let change = ChangeBoardTask::new(Arc::clone(&store), Arc::clone(&clock))?;
        let fold_back = FoldBackRun::new(Arc::clone(&store), Arc::clone(&clock), cells)?;
        Ok(Self {
            store,
            clock,
            swarm,
            change,
            fold_back,
        })
    }

    /// One look at `kept`, its board first, then its swarm, then its
    /// claim; `watch` carries what a look needs of the ones before, and a
    /// fold-back is handed to `recorder` when there is one.
    pub fn look(
        &self,
        kept: &KeptRun,
        watch: &mut Watch,
        recorder: Option<&dyn FoldBackRecorder>,
    ) -> Look {
        // A board unread this time renews nothing: the next look reads it.
        let claim_left = match self.on_board(kept) {
            Ok(OnBoard::Open { claim_left }) => claim_left,
            Ok(OnBoard::Ended) => return Look::ending(KeeperEnd::Ended),
            Ok(OnBoard::Gone) => return Look::ending(KeeperEnd::Gone),
            Err(_) => None,
        };
        let sight = self
            .swarm
            .board(&kept.environment_ref)
            .ok()
            .and_then(|projection| sight_of(&projection));
        match sight {
            Some(SwarmSight::Ended(view)) => {
                watch.stalled_since = None;
                let folded = self
                    .fold_back
                    .execute_recorded(&kept.task, kept.run, &view, recorder);
                match folded {
                    Ok(FoldedBack::Folded(_)) => return Look::ending(KeeperEnd::Folded),
                    Ok(FoldedBack::Ended(_)) => return Look::ending(KeeperEnd::Ended),
                    Err(ProjectBoardError::NotFound(_)) => return Look::ending(KeeperEnd::Gone),
                    // Tried again at the next look (an allowlist): anything
                    // else is final, and its record says why.
                    Err(
                        ProjectBoardError::Contended { .. }
                        | ProjectBoardError::Store(BoardStoreError::Unavailable(_))
                        | ProjectBoardError::Clock(_),
                    ) => {}
                    Err(_) => return Look::ending(KeeperEnd::FoldBackRefused),
                }
            }
            Some(SwarmSight::Going {
                coordinator_alive: true,
            }) => watch.stalled_since = None,
            Some(SwarmSight::Going {
                coordinator_alive: false,
            })
            | None => {
                let now = self.clock.elapsed();
                let since = *watch.stalled_since.get_or_insert(now);
                if now.saturating_sub(since) >= STALL_LIMIT {
                    watch.renewing = false;
                    return Look::ending(KeeperEnd::SwarmLost);
                }
            }
        }
        let renewal = match (watch.renewing, claim_left) {
            (true, Some(left_s)) if left_s < RENEW_BELOW_SECONDS => {
                Some(self.renew(kept, watch, left_s))
            }
            (true | false, _) => None,
        };
        Look { renewal, end: None }
    }

    /// The claim renewed. Refused because another holds it now, or none
    /// does, renewing is no longer the keeper's; any other refusal is
    /// tried again at the next look.
    fn renew(&self, kept: &KeptRun, watch: &mut Watch, left_s: i64) -> Renewal {
        match self.change.execute(&kept.task, &BoardOp::Renew) {
            Ok(_) => Renewal::Renewed { left_s },
            Err(error) => {
                if matches!(
                    error,
                    ProjectBoardError::Rule(BoardRuleError::NotHolder | BoardRuleError::NoClaim)
                ) {
                    watch.renewing = false;
                }
                Renewal::Refused { left_s, error }
            }
        }
    }

    /// The kept run as the board holds it now.
    fn on_board(&self, kept: &KeptRun) -> Result<OnBoard, ProjectBoardError> {
        let board = self.store.read()?;
        let task = match board_view::task(&board, &kept.task) {
            Ok(task) => task,
            Err(ProjectBoardError::NotFound(_)) => return Ok(OnBoard::Gone),
            Err(error) => return Err(error),
        };
        let runs = board_view::runs_of(&board, &kept.task)?;
        let Some(run) = runs.iter().find(|run| run.fields().number == kept.run) else {
            return Ok(OnBoard::Gone);
        };
        if run.fields().outcome.is_some() {
            return Ok(OnBoard::Ended);
        }
        let now = self.clock.now()?;
        let claim_left = task
            .fields()
            .claim
            .as_ref()
            .map(|claim| claim.expires.seconds_since(&now));
        Ok(OnBoard::Open { claim_left })
    }
}

#[cfg(test)]
#[path = "keep_started_run_tests.rs"]
mod tests;
