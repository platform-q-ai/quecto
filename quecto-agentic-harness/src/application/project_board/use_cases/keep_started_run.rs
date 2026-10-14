//! Keeps a started run (S4d, #2479): one look at a time, as
//! [`keeper`](crate::application::project_board::keeper) says. The run's
//! keeper calls [`KeepStartedRun::look`] as each look asks, until a look
//! ends the keeping or its session ends; what a look did is the keeper's
//! to record, and what it says the session's to hear.
//!
//! Blocking: the store's and the swarm's reads, and a renewal's, a
//! fold-back's or the run's naming of its environment, hold the calling
//! thread.
use std::sync::Arc;

use super::board_change::{checked_writer, commit_run, task_on};
use super::{ChangeBoardTask, FoldBackRun, board_view};
use crate::application::project_board::dto::{
    BoardOp, FoldedBack, ProjectBoardError, SwarmBoardSight, SwarmStage,
};
use crate::application::project_board::fold_back::SwarmRunView;
use crate::application::project_board::keeper::{
    KeeperEnd, KeeperNotice, KeptRun, Look, PAUSED_LIMIT, PAUSED_PERIOD, RENEW_BELOW_SECONDS,
    Renewal, STALL_LIMIT, START_GRACE, Watch,
};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardStoreError, BoardTextCells, FoldBackRecorder,
    ProjectBoardStore, StartedSwarm,
};
use crate::domain::project_board::services::runs::kept_in;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

pub struct KeepStartedRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    swarm: Arc<dyn StartedSwarm>,
    change: ChangeBoardTask,
    fold_back: FoldBackRun,
}

/// The kept run as the board holds it.
enum OnBoard {
    /// Going: its task's claim expiry, when it has one, and whether the
    /// run names its environment.
    Open {
        claim_expires: Option<Timestamp>,
        named: bool,
    },
    /// Ended already.
    Ended,
    /// The task, or the run, is not on the board.
    Gone,
}

/// What became of a fold-back.
enum Fold {
    Ends(KeeperEnd),
    /// Not this time: tried again at the next look.
    Again,
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

    /// One look at `kept`: its claim, then its swarm. `watch` carries what
    /// a look needs of the ones before, and a fold-back is handed to
    /// `recorder` when there is one.
    pub fn look(
        &self,
        kept: &KeptRun,
        watch: &mut Watch,
        recorder: Option<&dyn FoldBackRecorder>,
    ) -> Look {
        let now = self.clock.elapsed();
        let began = *watch.began.get_or_insert(now);
        let renewal = match self.claim(kept, watch) {
            Ok(renewal) => renewal,
            Err(end) => return Look::ending(end),
        };
        let mut look = Look {
            renewal,
            ..Look::default()
        };
        let sight = match self.swarm.board(&kept.environment_ref) {
            SwarmBoardSight::Board(sight) => sight,
            SwarmBoardSight::Gone => return self.gone(kept, watch, recorder, look),
            SwarmBoardSight::Unreadable(_) => {
                return self.stalled(watch, now, began, look);
            }
        };
        // The run first sighted is the one kept: another there later is
        // the swarm made again, and so gone.
        match (&sight.run_id, &watch.run_id) {
            (Some(run), Some(bound)) if run != bound => {
                return self.gone(kept, watch, recorder, look);
            }
            (Some(run), None) => watch.run_id = Some(run.clone()),
            (Some(_), Some(_)) | (None, _) => {}
        }
        match sight.stage {
            SwarmStage::Ended(view) => {
                // A view cut short is never folded back.
                if sight.truncated {
                    return Look::ending(KeeperEnd::Truncated);
                }
                return match self.fold(kept, &view, recorder) {
                    Fold::Ends(end) => Look::ending(end),
                    Fold::Again => look,
                };
            }
            SwarmStage::Paused { proposed } => {
                watch.proposed = proposed.filter(|_| !sight.truncated);
                let since = *watch.paused_since.get_or_insert(now);
                look.next = PAUSED_PERIOD;
                if watch.renewing && now.saturating_sub(since) >= PAUSED_LIMIT {
                    watch.renewing = false;
                    look.notice = Some(KeeperNotice::PausedTooLong);
                }
            }
            SwarmStage::Going => {
                (watch.paused_since, watch.proposed) = (None, None);
            }
        }
        match (sight.run_id.is_some(), sight.coordinator_alive) {
            (true, true) => {
                watch.stalled_since = None;
                look
            }
            (true, false) | (false, false) => self.stalled(watch, now, began, look),
            (false, true) => look,
        }
    }

    /// The claim, renewed when less than [`RENEW_BELOW_SECONDS`] are left of
    /// it as last read, after the board is read again; or how the keeping
    /// ends, found there. A board unread renews nothing.
    fn claim(&self, kept: &KeptRun, watch: &mut Watch) -> Result<Option<Renewal>, KeeperEnd> {
        let left =
            |expires: &Timestamp| self.clock.now().ok().map(|now| expires.seconds_since(&now));
        let due = match &watch.claim_expires {
            None => true,
            Some(expires) => left(expires).is_some_and(|left| left < RENEW_BELOW_SECONDS),
        };
        let read = !watch.adopted || (watch.renewing && due);
        if !read {
            return Ok(None);
        }
        let named = match self.on_board(kept) {
            Ok(OnBoard::Ended) => return Err(KeeperEnd::Ended),
            Ok(OnBoard::Gone) => return Err(KeeperEnd::Gone),
            Err(_) => return Ok(None),
            Ok(OnBoard::Open {
                claim_expires,
                named,
            }) => {
                watch.claim_expires = claim_expires;
                named
            }
        };
        // A run not named yet is named now; a refusal is tried again at
        // the next read.
        watch.adopted = match named {
            true => true,
            false => self.adopt(kept).is_ok(),
        };
        let Some(left_s) = watch.claim_expires.as_ref().and_then(left) else {
            return Ok(None);
        };
        match watch.renewing && left_s < RENEW_BELOW_SECONDS {
            true => Ok(Some(self.renew(kept, watch, left_s))),
            false => Ok(None),
        }
    }

    /// The claim renewed. Refused as contended, by a store that could not
    /// be reached or by the clock, it is tried again at the next look (an
    /// allowlist); refused otherwise, the renewing stops.
    fn renew(&self, kept: &KeptRun, watch: &mut Watch, left_s: i64) -> Renewal {
        match self.change.execute(&kept.task, &BoardOp::Renew) {
            Ok(task) => {
                watch.claim_expires = task.fields().claim.as_ref().map(|c| c.expires.clone());
                Renewal::Renewed { left_s }
            }
            Err(error) => {
                watch.renewing = matches!(
                    error,
                    ProjectBoardError::Contended { .. }
                        | ProjectBoardError::Store(BoardStoreError::Unavailable(_))
                        | ProjectBoardError::Clock(_)
                );
                Renewal::Refused { left_s, error }
            }
        }
    }

    /// The run's environment named on the board, so a later session of its
    /// holder can take the keeping up.
    fn adopt(&self, kept: &KeptRun) -> Result<(), ProjectBoardError> {
        let message = format!(
            "board: run {} of {} kept in {}",
            kept.run,
            kept.task.as_str(),
            kept.environment_ref
        );
        commit_run(&*self.store, &*self.clock, &message, None, |board, _| {
            task_on(board, &kept.task)?;
            let runs = board_view::runs_of(board, &kept.task)?;
            let run = runs
                .into_iter()
                .find(|run| run.fields().number == kept.run)
                .ok_or(ProjectBoardError::NotFound(kept.task.clone()))?;
            Ok(vec![BoardArtifact::Run(kept_in(
                run,
                &kept.environment_ref,
            )?)])
        })
        .map(|_| ())
    }

    /// The swarm is gone: what it proposed while paused is folded back;
    /// with nothing proposed, it is given up.
    fn gone(
        &self,
        kept: &KeptRun,
        watch: &Watch,
        recorder: Option<&dyn FoldBackRecorder>,
        look: Look,
    ) -> Look {
        let Some(view) = &watch.proposed else {
            return Look::ending(KeeperEnd::SwarmLost);
        };
        match self.fold(kept, view, recorder) {
            Fold::Ends(end) => Look::ending(end),
            Fold::Again => look,
        }
    }

    /// The swarm stalled this look: given up once stalled for
    /// [`STALL_LIMIT`] since it was sighted, or, never sighted, for
    /// [`START_GRACE`] since the keeping began.
    fn stalled(
        &self,
        watch: &mut Watch,
        now: std::time::Duration,
        began: std::time::Duration,
        look: Look,
    ) -> Look {
        let (since, limit) = match watch.run_id {
            Some(_) => (*watch.stalled_since.get_or_insert(now), STALL_LIMIT),
            None => (began, START_GRACE),
        };
        match now.saturating_sub(since) >= limit {
            true => Look::ending(KeeperEnd::SwarmLost),
            false => look,
        }
    }

    /// The run folded back from `view`.
    fn fold(
        &self,
        kept: &KeptRun,
        view: &SwarmRunView,
        recorder: Option<&dyn FoldBackRecorder>,
    ) -> Fold {
        let folded = self
            .fold_back
            .execute_recorded(&kept.task, kept.run, view, recorder);
        match folded {
            Ok(FoldedBack::Folded(_)) => Fold::Ends(KeeperEnd::Folded),
            Ok(FoldedBack::Ended(_)) => Fold::Ends(KeeperEnd::Ended),
            Err(ProjectBoardError::NotFound(_)) => Fold::Ends(KeeperEnd::Gone),
            // Tried again at the next look (an allowlist): anything else
            // is final, and its record says why.
            Err(
                ProjectBoardError::Contended { .. }
                | ProjectBoardError::Store(BoardStoreError::Unavailable(_))
                | ProjectBoardError::Clock(_),
            ) => Fold::Again,
            Err(_) => Fold::Ends(KeeperEnd::FoldBackRefused),
        }
    }

    /// The kept run as the board holds it now (a fetch).
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
        Ok(OnBoard::Open {
            claim_expires: task.fields().claim.as_ref().map(|c| c.expires.clone()),
            named: run.fields().environment.as_deref() == Some(kept.environment_ref.as_str()),
        })
    }
}

#[cfg(test)]
#[path = "keep_started_run_tests.rs"]
mod tests;
