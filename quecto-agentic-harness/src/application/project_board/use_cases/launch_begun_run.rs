//! Launches a begun start's swarm coordinator with its brief (epic #2705
//! B8), the client half of a start: on this machine's board after
//! [`super::BeginBoardRun`] wrote it, or after a board server began it.
//!
//! - **A failed launch is undone** where the start was begun
//!   ([`BegunStarts`]): the run folded back as failed, which moves the task
//!   back as the start's undo does. The refusal says how far that got; a
//!   run left open is for `end_run`.
//! - **Cancel.** A call cancelled after the begin launches nothing, and one
//!   cancelled during the launch stops it; either is undone the same way,
//!   as is a launch that panicked.
//! - **Never under a running swarm.** A coordinator the launch could not
//!   stop, or one this front end sees running ([`RunLiveness`]), leaves the
//!   start as it is: its run open, the claim held.
//!
//! Blocking: the launch and the undo hold the calling thread.
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::start_board_task::{launch_failed, undone_task};
use crate::application::agent_turn::agent_loop::tool_panic_scope::catch_in_call_site;
use crate::application::project_board::dto::{ProjectBoardError, RunAfterUndo};
use crate::application::project_board::ports::{
    BegunStarts, CoordinatorLaunch, LaunchRefused, LaunchedCoordinator, RunLiveness,
};
use crate::application::project_board::start::{Begun, StartFacts, StartUndo};

/// Why a start launched nothing when its call was cancelled after its
/// write.
const CANCELLED: &str = "the call was cancelled before the coordinator launched";

/// Why a start with no launcher launched nothing.
const UNLAUNCHED: &str = "this session may not launch a coordinator";

/// Why a start whose launch panicked launched nothing.
const PANICKED: &str = "the coordinator's launch failed unexpectedly (it panicked)";

pub struct LaunchBegunRun {
    /// Where a begun start is undone.
    runs: Arc<dyn BegunStarts>,
    /// `None`: this session may not spawn, so nothing is started.
    launcher: Option<Arc<dyn CoordinatorLaunch>>,
    /// Set when the call this start serves was cancelled.
    cancelled: Option<Arc<AtomicBool>>,
    /// Whether the run's coordinator still runs here, asked before an
    /// undo (`None`: this front end can see none).
    liveness: Option<Arc<dyn RunLiveness>>,
}

impl LaunchBegunRun {
    pub fn new(
        runs: Arc<dyn BegunStarts>,
        launcher: Option<Arc<dyn CoordinatorLaunch>>,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            runs,
            launcher,
            cancelled,
            liveness: None,
        }
    }

    /// This launch, asking `liveness` whether a coordinator still runs
    /// before it undoes anything (`None`: it can see none).
    #[must_use]
    pub fn seeing(self, liveness: Option<Arc<dyn RunLiveness>>) -> Self {
        Self { liveness, ..self }
    }

    /// Whether this session may launch now: asked before a start writes
    /// anything.
    ///
    /// # Errors
    /// No launcher, or one whose session may not spawn now.
    pub fn may_launch(&self) -> Result<(), ProjectBoardError> {
        let launcher = self.launcher.as_ref();
        match launcher.is_some_and(|launcher| launcher.may_launch()) {
            true => Ok(()),
            false => Err(ProjectBoardError::NoLauncher),
        }
    }

    /// Whether the call this start serves was cancelled: a start asks
    /// before it writes, and a cancelled one writes nothing.
    pub fn cancelled(&self) -> bool {
        let cancelled = self.cancelled.as_deref();
        cancelled.is_some_and(|cancelled| cancelled.load(Ordering::SeqCst))
    }

    /// `begun`'s coordinator launched, noting in `facts`, when given, what
    /// the begin saw, how long the launch took and, for one undone, where
    /// its undo left the run.
    ///
    /// # Errors
    /// The launch failed, or the call was cancelled: the start undone as
    /// far as it could be.
    pub fn launch<W>(
        &self,
        begun: Begun<W>,
        mut facts: Option<&mut StartFacts>,
    ) -> Result<(Begun<W>, LaunchedCoordinator), ProjectBoardError> {
        if let Some(facts) = facts.as_deref_mut() {
            *facts = begun.noted.clone();
        }
        // A call cancelled after its write launches nothing: it is undone,
        // as is a start with no launcher (`may_launch` refuses one first).
        let never = AtomicBool::new(false);
        let cancel = self.cancelled.as_deref().unwrap_or(&never);
        let launcher = match (self.launcher.as_ref(), cancel.load(Ordering::SeqCst)) {
            (Some(launcher), false) => launcher,
            (Some(_), true) => {
                return Err(self.undone(&begun, LaunchRefused::because(CANCELLED), facts));
            }
            (None, _) => {
                return Err(self.undone(&begun, LaunchRefused::because(UNLAUNCHED), facts));
            }
        };
        let before = facts.as_ref().and_then(|_| self.runs.elapsed());
        // A launch that panics is undone as one that failed: the write it
        // follows is never left behind. The catch holds while the call's
        // scope is open; a panic after a cancelled call's scope closed (its
        // work left running) is outside any scope, and the hook ends the
        // process, as for any carried work (`call_work`'s limit).
        let launched =
            catch_in_call_site(AssertUnwindSafe(|| launcher.launch(&begun.launch, cancel)))
                .unwrap_or_else(|_| Err(LaunchRefused::because(PANICKED)));
        if let (Some(facts), Some(before), Some(now)) =
            (facts.as_mut(), before, self.runs.elapsed())
        {
            let took = now.saturating_sub(before);
            facts.launch_us = Some(u64::try_from(took.as_micros()).unwrap_or(u64::MAX));
        }
        match launched {
            Ok(coordinator) => Ok((begun, coordinator)),
            Err(refused) => Err(self.undone(&begun, refused, facts)),
        }
    }

    /// The refusal of a start whose coordinator did not launch, once the
    /// start is undone as far as it can be: its run folded back as failed,
    /// which moves its task back as the start's undo does, or found ended
    /// already. A coordinator that may still be running (one that came up
    /// wrong and could not be stopped, or one seen running here) leaves the
    /// start as it is: its run open, for `end_run` once it is stopped, and
    /// the claim held. Noted in `facts`, when given, with the undo's
    /// fold-back.
    fn undone<W>(
        &self,
        begun: &Begun<W>,
        refused: LaunchRefused,
        facts: Option<&mut StartFacts>,
    ) -> ProjectBoardError {
        let LaunchRefused {
            reason,
            still_running,
        } = refused;
        let (task, run) = (&begun.launch.task, begun.launch.run);
        // A coordinator that came up anyway (a launch that panicked, was
        // cancelled or timed out after spawn began) may still be running
        // where this front end can see it: undone the same as one the
        // launch said it could not stop.
        let still_running = still_running.or_else(|| {
            let liveness = self.liveness.as_ref();
            liveness.and_then(|liveness| liveness.running_coordinator(task, run))
        });
        let undo = begun.undo(&reason);
        let (run_state, fold_back, refused) = match &still_running {
            // Never fold back, nor release, under a swarm that may run.
            Some(_) => (RunAfterUndo::Open, None, None),
            None => self.runs.undo(&undo, facts.is_some()),
        };
        // A board that refused the undo says why, beside the launch's reason.
        let reason = match refused {
            Some(why) => format!("{}; the board refused its undo: {why}", undo.reason),
            None => undo.reason.clone(),
        };
        let undo = StartUndo { reason, ..undo };
        if let Some(facts) = facts {
            facts.run_state = Some(run_state);
            facts.released = undone_task(run_state, begun.from).0;
            facts.fold_back = fold_back;
        }
        launch_failed(&undo, run_state, still_running)
    }
}
