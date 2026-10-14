//! A started run kept by the agent session that started it (S4d, #2479):
//! after a start through the agent tool, the run's keeper looks at it
//! every [`KEEPER_PERIOD`](crate::application::project_board::keeper::KEEPER_PERIOD) on a thread of its own
//! ([`KeepStartedRun::look`]) until a look ends the keeping or the
//! session ends.
//!
//! Its records, only while the session's event log is on: each fold-back
//! its `board_fold_back` ([`FoldBackCall`]), and its claim's renewals,
//! consecutive successes folded into one `board_claim_renewals` record
//! with the least time the claim had left before one, and each refusal a
//! record of its own, with its answer's code (`swarm_lost` for a swarm
//! given up). Kinds and counts only, as every board record.
//!
//! The run's swarm is read through the session's live board view
//! ([`SessionSwarmBoards`]), the same `ReadSwarmBoard` that answers
//! `get_swarm_board`.
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;

use super::answer::error_code;
use super::fold_back_call::FoldBackCall;
use super::recording::refusal_parts;
use crate::application::environments::use_cases::ReadSwarmBoard;
use crate::application::project_board::keeper::{KeeperEnd, KeptRun, Renewal, Watch};
use crate::application::project_board::ports::{FoldBackRecorder, StartedSwarm};
use crate::application::project_board::telemetry::BoardRecording;
use crate::application::project_board::use_cases::KeepStartedRun;
use crate::domain::project_board::value_objects::telemetry::{
    ClaimRenewalsRecord, OpOutcome, StaticKind, TaskRef,
};

/// The end of the session whose keepers watch it.
#[derive(Debug, Default)]
pub struct SessionEnd {
    ended: Mutex<bool>,
    changed: Condvar,
}

impl SessionEnd {
    /// Ends the session: every keeper waiting on it stops.
    pub fn end(&self) {
        *self.ended.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_all();
    }

    /// Waits `period`, unless the session ends first: whether it goes on.
    pub fn wait(&self, period: Duration) -> bool {
        let ended = self.ended.lock().unwrap_or_else(PoisonError::into_inner);
        let (ended, _) = self
            .changed
            .wait_timeout_while(ended, period, |ended| !*ended)
            .unwrap_or_else(PoisonError::into_inner);
        !*ended
    }
}

/// A session's keeping: the end its keepers watch, the swarm boards they
/// read, and how often they look (the keeper's period, but in tests).
#[derive(Clone)]
pub struct SessionKeeper {
    pub end: Arc<SessionEnd>,
    pub swarm: Arc<dyn StartedSwarm>,
    pub period: Duration,
}

impl std::fmt::Debug for SessionKeeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionKeeper").finish_non_exhaustive()
    }
}

impl SessionKeeper {
    /// Keeps `kept` with `keeper` on a thread of its own, recording in
    /// `recording` when there is one. A keeper that cannot be started
    /// leaves the run for `end_run`, and says so on the trace.
    pub fn keep(
        &self,
        keeper: KeepStartedRun,
        kept: KeptRun,
        recording: Option<Arc<BoardRecording>>,
    ) {
        let (end, period) = (Arc::clone(&self.end), self.period);
        let spawned = std::thread::Builder::new()
            .name("board-keeper".into())
            .spawn(move || kept_until_done(&keeper, &kept, &end, recording, period));
        if let Err(error) = spawned {
            tracing::warn!(%error, "the started run's keeper did not start");
        }
    }
}

/// Keeps `kept` with `keeper`, one look every `period`, until a look ends
/// the keeping or `session` ends; how it ended.
pub fn kept_until_done(
    keeper: &KeepStartedRun,
    kept: &KeptRun,
    session: &SessionEnd,
    recording: Option<Arc<BoardRecording>>,
    period: Duration,
) -> KeeperEnd {
    let fold = recording.clone().map(FoldBackCall::new);
    let mut renewals = recording.map(|recording| Renewals::new(recording, kept));
    let mut watch = Watch::default();
    let end = loop {
        if !session.wait(period) {
            break KeeperEnd::SessionEnded;
        }
        let recorder = fold.as_ref().map(|fold| fold as &dyn FoldBackRecorder);
        let look = keeper.look(kept, &mut watch, recorder);
        if let (Some(renewals), Some(renewal)) = (renewals.as_mut(), &look.renewal) {
            renewals.note(renewal);
        }
        if let Some(end) = look.end {
            break end;
        }
    };
    if let Some(renewals) = renewals.as_mut() {
        if end == KeeperEnd::SwarmLost {
            renewals.lost();
        }
        renewals.flush();
    }
    end
}

/// A keeper's renewals on their way to its records.
struct Renewals {
    recording: Arc<BoardRecording>,
    task: TaskRef,
    run: u64,
    /// Successes since the last record, and the least time left before one.
    renewed: u64,
    min_left_s: Option<i64>,
}

impl Renewals {
    fn new(recording: Arc<BoardRecording>, kept: &KeptRun) -> Self {
        Self {
            recording,
            task: TaskRef::of(&kept.task),
            run: u64::from(kept.run),
            renewed: 0,
            min_left_s: None,
        }
    }

    /// A success is folded with the ones before; a refusal ends them,
    /// then is a record of its own.
    fn note(&mut self, renewal: &Renewal) {
        match renewal {
            Renewal::Renewed { left_s } => {
                self.renewed = self.renewed.saturating_add(1);
                self.min_left_s = Some(self.min_left_s.map_or(*left_s, |min| min.min(*left_s)));
            }
            Renewal::Refused { left_s, error } => {
                self.flush();
                let outcome = OpOutcome::Refused {
                    code: StaticKind::new(error_code(error).as_str()),
                    rule: refusal_parts(error).0.map(StaticKind::new),
                };
                self.write(outcome, 0, Some(*left_s));
            }
        }
    }

    /// The swarm given up as lost: a refusal of its own.
    fn lost(&mut self) {
        self.flush();
        let outcome = OpOutcome::Refused {
            code: StaticKind::new("swarm_lost"),
            rule: None,
        };
        self.write(outcome, 0, None);
    }

    /// The successes since the last record, when there were any.
    fn flush(&mut self) {
        if self.renewed > 0 {
            let (renewed, min_left_s) = (self.renewed, self.min_left_s);
            self.write(OpOutcome::Ok, renewed, min_left_s);
            (self.renewed, self.min_left_s) = (0, None);
        }
    }

    fn write(&self, outcome: OpOutcome, renewals: u64, min_left_s: Option<i64>) {
        self.recording.record_claim_renewals(ClaimRenewalsRecord {
            task: self.task.clone(),
            run: self.run,
            outcome,
            renewals,
            min_left_s,
        });
    }
}

/// The session's swarm boards, read as its live board view reads them
/// (`get_swarm_board`), from a keeper's own thread: the read runs on the
/// session's runtime.
pub struct SessionSwarmBoards {
    boards: Arc<dyn Fn() -> Option<Arc<ReadSwarmBoard>> + Send + Sync>,
    runtime: tokio::runtime::Handle,
}

impl SessionSwarmBoards {
    /// The boards `boards` reaches (none until composition installed
    /// them), read on `runtime`.
    pub fn new(
        boards: Arc<dyn Fn() -> Option<Arc<ReadSwarmBoard>> + Send + Sync>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self { boards, runtime }
    }
}

impl StartedSwarm for SessionSwarmBoards {
    fn board(&self, environment_ref: &str) -> Result<Value, String> {
        // Blocking on the runtime from one of its own workers would stall
        // it: only a thread off the runtime reads (an allowlist).
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("a swarm board is read off the async runtime".into());
        }
        let Some(boards) = (self.boards)() else {
            return Err("swarm boards are unavailable in this session".into());
        };
        self.runtime
            .block_on(boards.board(environment_ref))
            .map(|snapshot| snapshot.into_value())
            .map_err(|refusal| refusal.to_string())
    }
}

#[cfg(test)]
#[path = "keeper_tests.rs"]
mod tests;
