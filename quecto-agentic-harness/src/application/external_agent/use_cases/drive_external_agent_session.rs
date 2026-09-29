//! Drive one claude-code member's session (#2287): start its agent, admit
//! prompts with quecto's busy semantics, fold the agent's stream, interrupt
//! its turns and end the member.
//!
//! - `prompt` starts a turn when idle; while a turn runs it needs a
//!   [`StreamingBehavior`]: `Steer` is written at once, `FollowUp` waits
//!   (at most [`FOLLOW_UP_QUEUE_CAPACITY`]) for the turn's end, which
//!   starts the next one. A session turn ends once claude's results have
//!   named every user turn written into it (a steer claude ran as a turn
//!   of its own included): until then nothing new starts.
//! - [`DriveExternalAgentSession::next_step`] reads and folds the stream;
//!   its caller loops on it. A turn whose last event was a skipped line (it
//!   may have been its `result`) is given up once the stream stays quiet
//!   for [`ExternalAgentSessionSettings::skipped_line_grace`]: it is
//!   interrupted, like an aborted one. The reader waits for the next event
//!   or the deadline then due; an abort or a `close` that changes what is
//!   due wakes a reader already waiting, which waits again for the new
//!   one, so a wedged agent that writes nothing more cannot hold it.
//! - `abort` is quecto's (`handle_abort`): it drops the follow-ups and
//!   answers at once; a running turn is interrupted. The member lives on.
//! - An interrupted turn keeps the session busy, writing nothing, until
//!   every result it owes has come or been withdrawn; if that takes longer
//!   than [`ExternalAgentSessionSettings::interrupt_grace`], the interrupt
//!   cannot be written, or claude refuses it while a result is owed,
//!   claude's state is unknown and the member is ended. So it is when a
//!   steer was taken on init's word and an id-less success follows.
//! - `close` ends the member: its turn, its follow-ups and its process. A
//!   reader or a writer waiting on it is released. It takes no write gate
//!   (a write blocked on a full pipe must not hold it up), so every
//!   decision taken after an await allows for the member having ended.
//!
//! Two bounds are heuristics, not claude's word:
//! - The lost-turn timer: a skipped line followed by
//!   [`ExternalAgentSessionSettings::skipped_line_grace`] of silence is
//!   taken for a lost `result`. A turn that really goes on generating in
//!   silence for longer than that after a skipped line (a long tool run)
//!   is interrupted by mistake; only that turn is lost, and the member
//!   lives on if claude answers the interrupt.
//! - The owed-result bound: an interrupted turn has
//!   [`ExternalAgentSessionSettings::interrupt_grace`] to deliver every
//!   result it owes (or have them withdrawn). A turn slower than that to
//!   stop (a tool that ignores the abort) ends the member, whose state is
//!   then unknown.
//!
//! Writes are serialised (a follow-up's turn cannot be overtaken by a
//! steer), and every decision and effect is a [`SessionRecord`]. Time is
//! the [`ExternalAgentClock`] port's.
//!
//! [`FOLLOW_UP_QUEUE_CAPACITY`]: crate::application::external_agent::dto::FOLLOW_UP_QUEUE_CAPACITY

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::application::external_agent::dto::{
    AbortOutcome, AgentClockInstant, ExecutionState, ExternalAgentExit,
    ExternalAgentSessionSettings, FinalReport, ProjectedMessage, PromptAccepted, SessionPhase,
    SessionRecord, SessionRefusal, SessionStep, SessionView, StreamingBehavior,
};
use crate::application::external_agent::ports::{
    ExternalAgentClock, ExternalAgentLauncher, ExternalAgentProcess, ExternalAgentTelemetry,
};
use crate::application::external_agent::session_core::{
    AbortDecision, Admission, SessionCore, Wait,
};
use crate::application::external_agent::session_telemetry::TurnCut;

type Process = Arc<dyn ExternalAgentProcess>;

/// What interrupting a turn came to.
enum Interrupt {
    /// It was written: the session waits for what the turn owes.
    Written,
    /// It could not be: the member was ended.
    Abandoned,
    /// The member ended meanwhile.
    MemberEnded,
}

/// One claude-code member's session.
pub struct DriveExternalAgentSession {
    launcher: Arc<dyn ExternalAgentLauncher>,
    telemetry: Arc<dyn ExternalAgentTelemetry>,
    clock: Arc<dyn ExternalAgentClock>,
    settings: ExternalAgentSessionSettings,
    core: Mutex<SessionCore>,
    process: Mutex<Option<Process>>,
    /// Held from a decision to write until the write is done (and across
    /// every fold, so a result is never folded before the user turn it
    /// names is recorded as owed).
    writes: tokio::sync::Mutex<()>,
    /// Set once the member has ended; releases every waiter.
    ended: tokio::sync::watch::Sender<bool>,
    /// Bumped whenever what the reader waits for changes other than by
    /// its own fold (an interrupt's deadline is set): a reader already
    /// waiting recomputes its wait.
    wait_changed: tokio::sync::watch::Sender<u64>,
}

impl DriveExternalAgentSession {
    pub fn new(
        launcher: Arc<dyn ExternalAgentLauncher>,
        telemetry: Arc<dyn ExternalAgentTelemetry>,
        clock: Arc<dyn ExternalAgentClock>,
        settings: ExternalAgentSessionSettings,
    ) -> Self {
        Self {
            launcher,
            telemetry,
            clock,
            settings,
            core: Mutex::default(),
            process: Mutex::default(),
            writes: tokio::sync::Mutex::new(()),
            ended: tokio::sync::watch::Sender::new(false),
            wait_changed: tokio::sync::watch::Sender::new(0),
        }
    }

    /// Start the member's agent; once only.
    pub async fn start(&self) -> Result<(), SessionRefusal> {
        let _writes = self.writes.lock().await;
        match self.core().phase {
            SessionPhase::NotStarted => {}
            SessionPhase::Idle
            | SessionPhase::Busy { .. }
            | SessionPhase::Interrupting { .. }
            | SessionPhase::Ended => {
                return Err(SessionRefusal::AlreadyStarted);
            }
        }
        match self.launcher.start(self.settings.launch.clone()).await {
            Ok(process) => {
                *self.slot() = Some(Arc::from(process));
                let mut core = self.core();
                core.projector.process_started();
                core.telemetry.process_started(self.clock.now());
                core.phase = SessionPhase::Idle;
                drop(core);
                self.record(SessionRecord::Started);
                Ok(())
            }
            Err(error) => {
                self.core().end();
                self.ended.send_replace(true);
                self.record(SessionRecord::StartRefused { kind: error.kind() });
                Err(SessionRefusal::Launch(error))
            }
        }
    }

    /// Deliver `text`: see the module's account of busy semantics.
    pub async fn prompt(
        &self,
        text: &str,
        behavior: Option<StreamingBehavior>,
    ) -> Result<PromptAccepted, SessionRefusal> {
        let _writes = self.writes.lock().await;
        let admitted = self.core().admit(text, behavior);
        let outcome = match admitted {
            Ok(Admission::Queued(accepted)) => Ok(accepted),
            Ok(Admission::Write(accepted)) => self.write(accepted, text).await.map(|()| accepted),
            Err(refusal) => Err(refusal),
        };
        self.record(match &outcome {
            Ok(accepted) => SessionRecord::PromptAccepted {
                accepted: *accepted,
                bytes: text.len(),
            },
            Err(refusal) => SessionRecord::PromptRefused {
                refusal: refusal.kind(),
                bytes: text.len(),
            },
        });
        outcome
    }

    pub async fn steer(&self, text: &str) -> Result<PromptAccepted, SessionRefusal> {
        self.prompt(text, Some(StreamingBehavior::Steer)).await
    }

    pub async fn follow_up(&self, text: &str) -> Result<PromptAccepted, SessionRefusal> {
        self.prompt(text, Some(StreamingBehavior::FollowUp)).await
    }

    /// quecto's abort: drop the follow-ups and interrupt the running turn,
    /// if one runs. The member lives on, unless the interrupt cannot be
    /// written.
    pub async fn abort(&self) -> Result<AbortOutcome, SessionRefusal> {
        let _writes = self.writes.lock().await;
        let decision = self.core().abort()?;
        let (turn, dropped_follow_ups, member_ended) = match decision {
            AbortDecision::Idle { dropped } => (None, dropped, false),
            AbortDecision::Interrupting { turn, dropped } => (Some(turn), dropped, false),
            AbortDecision::Interrupt { turn, dropped } => {
                let ended = match self.interrupt(turn, "abort").await {
                    Interrupt::Written => false,
                    Interrupt::Abandoned | Interrupt::MemberEnded => true,
                };
                (Some(turn), dropped, ended)
            }
        };
        self.record(SessionRecord::Aborted {
            turn,
            dropped_follow_ups,
        });
        Ok(AbortOutcome {
            turn,
            dropped_follow_ups,
            member_ended,
        })
    }

    /// End the member: its turn, its follow-ups and its agent's process.
    pub async fn close(&self) -> Result<AbortOutcome, SessionRefusal> {
        let (turn, dropped_follow_ups) = {
            let mut core = self.core();
            match core.phase {
                SessionPhase::Idle
                | SessionPhase::Busy { .. }
                | SessionPhase::Interrupting { .. } => core.end(),
                SessionPhase::NotStarted => return Err(SessionRefusal::NotStarted),
                SessionPhase::Ended => return Err(SessionRefusal::Ended),
            }
        };
        self.release_process();
        self.record(SessionRecord::Closed {
            turn,
            dropped_follow_ups,
        });
        Ok(AbortOutcome {
            turn,
            dropped_follow_ups,
            member_ended: true,
        })
    }

    /// Read and fold the agent's next event: `None` once the member has
    /// not started or has ended.
    pub async fn next_step(&self) -> Option<SessionStep> {
        loop {
            if let Some(step) = self.core().take_surfaced() {
                return Some(step);
            }
            let process = self.slot().clone()?;
            // An overdue interrupt is acted on before another event is
            // read: a stream that keeps talking cannot starve it.
            if self.core().interrupt_overdue(self.clock.now()).is_some()
                && let Some(step) = self.abandon_if_overdue().await
            {
                return Some(step);
            }
            // Subscribed before the wait is read: a change after this
            // point wakes the wait below, and one before it is in `wait`.
            let mut changed = self.wait_changed.subscribe();
            let wait = self.core().wait();
            let timer = self.timer(wait);
            // `None`: the timer ran out; `Some(None)`: the output ended.
            // Reading an event is cancel-safe: one left unread when the
            // wait changes is read on the next pass.
            let read = tokio::select! {
                biased;
                () = self.until_ended() => return None,
                // The sender lives as long as `self`: never an error.
                _ = changed.changed() => continue,
                event = process.next_event() => Some(event),
                () = self.clock.sleep(timer.unwrap_or_default()), if timer.is_some() => None,
            };
            match (read, wait) {
                (Some(Some(event)), _) => {
                    // A skipped line's grace runs from when it was read: a
                    // write holding the gate does not stretch it.
                    let now = self.clock.now();
                    let grace_end = self.deadline_after(self.settings.skipped_line_grace);
                    let _writes = self.writes.lock().await;
                    let folded = self.core().fold(&event, now, grace_end);
                    self.settle(folded.records, folded.follow_up).await;
                    // claude's state became unknown: the member ends, and
                    // the reader is told next.
                    if let Some(turn) = folded.abandon {
                        let abandoned = self.abandon(turn);
                        self.core().surface(abandoned);
                    }
                    return Some(SessionStep::Folded(folded.step));
                }
                (Some(None), _) => return self.output_ended(process).await,
                (None, Wait::Grace(_)) => {
                    if let Some(step) = self.lose_turn().await {
                        return Some(step);
                    }
                }
                (None, Wait::Until(_)) => {
                    if let Some(step) = self.abandon_if_overdue().await {
                        return Some(step);
                    }
                }
                // No timer runs without a wait: read on.
                (None, Wait::Event) => {}
            }
            // A turn that ended or answered meanwhile is not lost: read on.
        }
    }

    pub fn state(&self) -> SessionView {
        let core = self.core();
        SessionView {
            phase: core.phase,
            // A lost turn leaves the projection mid-turn: only a running
            // turn has a state other than idle.
            execution: match core.running_turn() {
                Some(_) => core.projector.state(),
                None => ExecutionState::Idle,
            },
            queued_follow_ups: core.queued(),
            totals: core.projector.session_totals(),
        }
    }

    /// The latest turn's report.
    pub fn report(&self) -> Option<FinalReport> {
        self.core().projector.report().cloned()
    }

    /// At most `count` messages from ordinal `start`.
    pub fn messages(&self, start: usize, count: usize) -> Vec<ProjectedMessage> {
        let core = self.core();
        let messages = core.projector.messages();
        let start = start.min(messages.len());
        let end = start.saturating_add(count).min(messages.len());
        messages[start..end].to_vec()
    }

    /// How long the reader may wait for an event before `wait` is due.
    fn timer(&self, wait: Wait) -> Option<Duration> {
        match wait {
            Wait::Event => None,
            Wait::Grace(deadline) | Wait::Until(deadline) => Some(Duration::from_millis(
                deadline.0.saturating_sub(self.clock.now().0),
            )),
        }
    }

    /// Give up the running turn, if its last event is still a skipped line:
    /// it is interrupted.
    async fn lose_turn(&self) -> Option<SessionStep> {
        let _writes = self.writes.lock().await;
        let turn = self.core().lost_turn(self.clock.now())?;
        match self.interrupt(turn, "lost").await {
            Interrupt::Written => Some(SessionStep::TurnLost { turn }),
            Interrupt::Abandoned => Some(SessionStep::Abandoned { turn }),
            Interrupt::MemberEnded => None,
        }
    }

    /// Interrupt running turn `turn`; the caller holds the write gate.
    /// Written, the session waits for what the turn owes.
    async fn interrupt(&self, turn: u64, cause: &'static str) -> Interrupt {
        let Some(process) = self.slot().clone() else {
            return Interrupt::MemberEnded;
        };
        let written = tokio::select! {
            biased;
            () = self.until_ended() => return Interrupt::MemberEnded,
            written = process.interrupt() => written,
        };
        drop(process);
        let deadline = self.deadline_after(self.settings.interrupt_grace);
        // `close` may have ended the member while the interrupt was being
        // written: then there is nothing to wait for, or to abandon.
        let mut core = self.core();
        match (written, core.ended()) {
            (_, true) => Interrupt::MemberEnded,
            (Ok(()), false) => {
                let waiting = core.interrupting(turn, deadline);
                assert!(waiting, "a member that has not ended waits");
                drop(core);
                // A reader waiting on the turn's events waits for the
                // deadline instead.
                self.wait_changed.send_modify(|n| *n = n.wrapping_add(1));
                self.record(SessionRecord::Interrupted { turn, cause });
                Interrupt::Written
            }
            (Err(_), false) => {
                drop(core);
                self.abandon(turn);
                Interrupt::Abandoned
            }
        }
    }

    fn deadline_after(&self, grace: Duration) -> AgentClockInstant {
        // Saturating: a grace past u64 milliseconds never runs out.
        let grace_ms: u64 = grace.as_millis().try_into().unwrap_or(!0);
        AgentClockInstant(self.clock.now().0.saturating_add(grace_ms))
    }

    /// End the member if the interrupted turn is past its deadline.
    async fn abandon_if_overdue(&self) -> Option<SessionStep> {
        let _writes = self.writes.lock().await;
        let turn = self.core().interrupt_overdue(self.clock.now())?;
        Some(self.abandon(turn))
    }

    /// Interrupted turn `turn` never answered, or could not be
    /// interrupted: claude's state is unknown, so the member is ended.
    fn abandon(&self, turn: u64) -> SessionStep {
        let (_, dropped_follow_ups) = self.core().end();
        self.release_process();
        self.record(SessionRecord::Abandoned {
            turn,
            dropped_follow_ups,
        });
        SessionStep::Abandoned { turn }
    }

    async fn output_ended(&self, process: Process) -> Option<SessionStep> {
        let exit = tokio::select! {
            biased;
            () = self.until_ended() => return None,
            exit = process.exited() => exit,
        };
        drop(process);
        let now = self.clock.now();
        let mut reported = Vec::new();
        let (turn, wall_ms) = {
            let mut core = self.core();
            if let Some(turn) = core.running_turn() {
                core.telemetry
                    .turn_ended(turn, TurnCut::Exited, now, &mut reported);
            }
            let wall_ms = core.telemetry.wall_ms(now);
            (core.end().0, wall_ms)
        };
        self.release_process();
        for record in reported {
            self.record(record);
        }
        if let Some(turn) = turn {
            self.record(SessionRecord::TurnEnded {
                turn,
                outcome: "exited",
                duration_ms: None,
                cost_micro_usd: 0,
            });
        }
        let (exit_code, signal) = match exit {
            ExternalAgentExit::Code(code) => (Some(code), None),
            ExternalAgentExit::Signal(signal) => (None, Some(signal)),
            ExternalAgentExit::Unobservable(_) => (None, None),
        };
        self.record(SessionRecord::Ended {
            clean: exit.is_clean(),
            exit_code,
            signal,
            wall_ms,
        });
        Some(SessionStep::Ended { turn, exit })
    }

    /// Drop the process and release every waiter: the last handle to go
    /// ends it.
    fn release_process(&self) {
        let process = self.slot().take();
        self.ended.send_replace(true);
        drop(process);
    }

    /// Record what a fold decided and start the follow-up it dequeued. A
    /// follow-up that cannot be written is recorded and surfaced to the
    /// reader; the member is idle.
    async fn settle(&self, records: Vec<SessionRecord>, follow_up: Option<(u64, String)>) {
        for record in &records {
            self.telemetry.record(record);
        }
        let Some((turn, text)) = follow_up else {
            return;
        };
        if let Err(refusal) = self.write(PromptAccepted::Started { turn }, &text).await {
            self.record(SessionRecord::FollowUpFailed {
                turn,
                bytes: text.len(),
                refusal: refusal.kind(),
            });
            self.core()
                .surface(SessionStep::FollowUpFailed { turn, refusal });
        }
    }

    /// Write `accepted`'s text to the agent, unless the member ends first;
    /// once written it is part of the conversation, and owed a result.
    async fn write(&self, accepted: PromptAccepted, text: &str) -> Result<(), SessionRefusal> {
        let process = self.slot().clone().ok_or(SessionRefusal::Ended)?;
        let written = tokio::select! {
            biased;
            () = self.until_ended() => Err(SessionRefusal::Ended),
            sent = process.send_user_turn(text) => sent.map_err(SessionRefusal::Input),
        };
        let mut core = self.core();
        match written {
            Ok(id) => {
                core.written(id, text);
                Ok(())
            }
            Err(refusal) => {
                core.write_failed(accepted);
                Err(refusal)
            }
        }
    }

    /// Resolves once the member has ended.
    async fn until_ended(&self) {
        let mut ended = self.ended.subscribe();
        // The sender lives as long as `self`: an error cannot happen here.
        let _ = ended.wait_for(|ended| *ended).await;
    }

    fn record(&self, record: SessionRecord) {
        self.telemetry.record(&record);
    }

    fn core(&self) -> std::sync::MutexGuard<'_, SessionCore> {
        self.core
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Process>> {
        self.process
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
#[path = "drive_external_agent_session_rig_tests.rs"]
mod test_rig;

#[cfg(test)]
#[path = "drive_external_agent_session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "drive_external_agent_session_interrupt_tests.rs"]
mod interrupt_tests;

#[cfg(test)]
#[path = "drive_external_agent_session_binding_tests.rs"]
mod binding_tests;

#[cfg(test)]
#[path = "drive_external_agent_session_wake_tests.rs"]
mod wake_tests;

#[cfg(test)]
#[path = "drive_external_agent_session_capability_tests.rs"]
mod capability_tests;

#[cfg(test)]
#[path = "drive_external_agent_session_telemetry_tests.rs"]
mod telemetry_tests;
