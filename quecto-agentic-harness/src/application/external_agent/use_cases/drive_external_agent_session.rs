//! Drive one claude-code member's session (#2287): start its agent, admit
//! prompts with quecto's busy semantics, fold the agent's stream, and end
//! the member.
//!
//! - `prompt` starts a turn when idle; while a turn runs it needs a
//!   [`StreamingBehavior`]: `Steer` is written at once and folds into the
//!   running turn, `FollowUp` waits (at most [`FOLLOW_UP_QUEUE_CAPACITY`])
//!   for the turn's end, which starts the next one.
//! - [`DriveExternalAgentSession::next_step`] reads and folds the stream;
//!   its caller loops on it. A turn that skipped a line (it may have been
//!   its `result`) is given up once the stream stays quiet for
//!   [`ExternalAgentSessionSettings::skipped_line_grace`].
//! - `abort` ends the member: the agent has no abort message on its input
//!   (owner decision, #2287), so its process is ended, its turn and its
//!   follow-ups with it. A reader or a writer waiting on it is released.
//!
//! Writes are serialised (a follow-up's turn cannot be overtaken by a
//! steer), and every decision and effect is a [`SessionRecord`].
//!
//! [`FOLLOW_UP_QUEUE_CAPACITY`]: crate::application::external_agent::dto::FOLLOW_UP_QUEUE_CAPACITY

use std::sync::{Arc, Mutex};

use crate::application::external_agent::dto::{
    AbortOutcome, ExecutionState, ExternalAgentSessionSettings, FinalReport, ProjectedMessage,
    PromptAccepted, SessionPhase, SessionRecord, SessionRefusal, SessionStep, SessionView,
    StreamingBehavior,
};
use crate::application::external_agent::ports::{
    ExternalAgentLauncher, ExternalAgentProcess, ExternalAgentTelemetry,
};
use crate::application::external_agent::session_core::{Admission, Folded, SessionCore};

type Process = Arc<dyn ExternalAgentProcess>;

/// One claude-code member's session.
pub struct DriveExternalAgentSession {
    launcher: Arc<dyn ExternalAgentLauncher>,
    telemetry: Arc<dyn ExternalAgentTelemetry>,
    settings: ExternalAgentSessionSettings,
    core: Mutex<SessionCore>,
    process: Mutex<Option<Process>>,
    /// Held from a decision to write until the write is done.
    writes: tokio::sync::Mutex<()>,
    /// Set once the member has ended; releases every waiter.
    ended: tokio::sync::watch::Sender<bool>,
}

impl DriveExternalAgentSession {
    pub fn new(
        launcher: Arc<dyn ExternalAgentLauncher>,
        telemetry: Arc<dyn ExternalAgentTelemetry>,
        settings: ExternalAgentSessionSettings,
    ) -> Self {
        Self {
            launcher,
            telemetry,
            settings,
            core: Mutex::default(),
            process: Mutex::default(),
            writes: tokio::sync::Mutex::new(()),
            ended: tokio::sync::watch::Sender::new(false),
        }
    }

    /// Start the member's agent; once only.
    pub async fn start(&self) -> Result<(), SessionRefusal> {
        let _writes = self.writes.lock().await;
        match self.core().phase {
            SessionPhase::NotStarted => {}
            SessionPhase::Idle | SessionPhase::Busy { .. } | SessionPhase::Ended => {
                return Err(SessionRefusal::AlreadyStarted);
            }
        }
        match self.launcher.start(self.settings.launch.clone()).await {
            Ok(process) => {
                *self.slot() = Some(Arc::from(process));
                let mut core = self.core();
                core.projector.process_started();
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

    /// End the member: its turn, its follow-ups and its agent's process.
    pub async fn abort(&self) -> Result<AbortOutcome, SessionRefusal> {
        let (turn, dropped_follow_ups) = {
            let mut core = self.core();
            match core.phase {
                SessionPhase::Idle | SessionPhase::Busy { .. } => core.end(),
                SessionPhase::NotStarted => return Err(SessionRefusal::NotStarted),
                SessionPhase::Ended => return Err(SessionRefusal::Ended),
            }
        };
        // Waiters drop their handles on the signal; the last one to go
        // ends the process.
        let process = self.slot().take();
        self.ended.send_replace(true);
        drop(process);
        let outcome = AbortOutcome {
            turn,
            dropped_follow_ups,
        };
        self.record(SessionRecord::Aborted {
            turn,
            dropped_follow_ups,
        });
        Ok(outcome)
    }

    /// Read and fold the agent's next event: `None` once the member has
    /// not started or has ended.
    pub async fn next_step(&self) -> Option<SessionStep> {
        loop {
            let process = self.slot().clone()?;
            let armed = self.core().grace_armed();
            // `None`: the grace ran out; `Some(None)`: the output ended.
            let read = tokio::select! {
                biased;
                () = self.until_ended() => return None,
                event = process.next_event() => Some(event),
                () = tokio::time::sleep(self.settings.skipped_line_grace), if armed => None,
            };
            match read {
                Some(Some(event)) => {
                    let _writes = self.writes.lock().await;
                    let folded = self.core().fold(&event);
                    self.settle(folded.records, folded.follow_up).await;
                    return Some(SessionStep::Folded(folded.step));
                }
                Some(None) => return self.output_ended(process).await,
                // A turn that ended meanwhile is not lost: read on.
                None => match self.lose_turn().await {
                    Some(step) => return Some(step),
                    None => continue,
                },
            }
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

    async fn lose_turn(&self) -> Option<SessionStep> {
        let _writes = self.writes.lock().await;
        let lost = self.core().lose_turn();
        let (turn, folded) = lost?;
        let Folded {
            records, follow_up, ..
        } = folded;
        self.settle(records, follow_up).await;
        Some(SessionStep::TurnLost { turn })
    }

    async fn output_ended(&self, process: Process) -> Option<SessionStep> {
        let exit = tokio::select! {
            biased;
            () = self.until_ended() => return None,
            exit = process.exited() => exit,
        };
        drop(process);
        let (turn, _) = self.core().end();
        let lost = self.slot().take();
        self.ended.send_replace(true);
        drop(lost);
        if let Some(turn) = turn {
            self.record(SessionRecord::TurnEnded {
                turn,
                outcome: "exited",
                duration_ms: None,
                cost_micro_usd: 0,
            });
        }
        self.record(SessionRecord::Ended {
            clean: exit.is_clean(),
        });
        Some(SessionStep::Ended { turn, exit })
    }

    /// Record what a fold decided and start the follow-up it dequeued.
    async fn settle(&self, records: Vec<SessionRecord>, follow_up: Option<(u64, String)>) {
        for record in &records {
            self.telemetry.record(record);
        }
        if let Some((turn, text)) = follow_up {
            // A failed write is the member's end, which the stream reports.
            let _ = self.write(PromptAccepted::Started { turn }, &text).await;
        }
    }

    /// Write `accepted`'s text to the agent, unless the member ends first;
    /// once written it is part of the conversation.
    async fn write(&self, accepted: PromptAccepted, text: &str) -> Result<(), SessionRefusal> {
        let process = self.slot().clone().ok_or(SessionRefusal::Ended)?;
        let written = tokio::select! {
            biased;
            () = self.until_ended() => Err(SessionRefusal::Ended),
            sent = process.send_user_turn(text) => sent.map_err(SessionRefusal::Input),
        };
        let mut core = self.core();
        match &written {
            Ok(()) => {
                core.projector.record_user_turn(text);
            }
            Err(_) => core.write_failed(accepted),
        }
        written
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
#[path = "drive_external_agent_session_tests.rs"]
mod tests;
