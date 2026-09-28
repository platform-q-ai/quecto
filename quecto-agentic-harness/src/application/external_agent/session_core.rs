//! The member session's bookkeeping (#2287): its phase, the follow-up
//! queue, the turn ordinals, the user turns claude still owes a result
//! for, and the projection. Pure: the session use case
//! ([`super::use_cases::DriveExternalAgentSession`]) holds one under its
//! lock and does the I/O each decision calls for.
//!
//! claude emits exactly one `result` per turn, naming every user turn it
//! consumed (`user_message_uuids`): a user turn written while a turn runs
//! is folded into it, or runs as a turn of its own once that ends. So a
//! session turn ends only once every user turn written into it has been
//! named by a result, or withdrawn by an interrupt: until then claude is
//! still working and nothing new is started.
//!
//! Invariants, asserted: at most one turn is in flight (a turn begins only
//! from `Idle`), turn ordinals strictly increase, a turn ends only when
//! nothing is owed, only a running turn is interrupted, and the queue never
//! holds more than [`FOLLOW_UP_QUEUE_CAPACITY`] follow-ups.

use std::collections::{BTreeSet, VecDeque};

use super::dto::{
    AgentClockInstant, FOLLOW_UP_QUEUE_CAPACITY, ProjectionStep, PromptAccepted, SessionPhase,
    SessionRecord, SessionRefusal, SessionStep, StreamingBehavior, TOOL_NAME_RECORD_BYTES,
    TurnOutcome, UserTurnId,
};
use super::projection::Projector;
use crate::domain::external_agent::stream::{AssistantContent, ExternalAgentEvent};
use crate::domain::external_agent::turn::{FailureKind, TurnEnd};

#[derive(Debug, Default)]
pub(crate) struct SessionCore {
    pub(crate) phase: SessionPhase,
    pub(crate) projector: Projector,
    follow_ups: VecDeque<String>,
    last_turn: u64,
    /// The last event was a skipped line of the running turn: it may have
    /// been its `result`. Any other event disarms it.
    skipped_last: bool,
    /// User turns written and not yet named by a result or withdrawn.
    owed: BTreeSet<UserTurnId>,
    /// Whether the agent names the turns its results consumed: learnt from
    /// the first result that does. Until then (an older CLI) a result is
    /// taken to answer everything written.
    names_turns: bool,
    /// The outcome of a result that did not end the turn (something was
    /// still owed): reported when the turn ends on a withdrawal.
    held: Option<TurnOutcome>,
    /// When an interrupted turn must have answered by.
    deadline: Option<AgentClockInstant>,
    /// Steps for the reader, returned before it reads again.
    surfaced: VecDeque<SessionStep>,
}

/// What a prompt calls for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Write the prompt to the agent; it is `accepted` once written.
    Write(PromptAccepted),
    /// It was queued: nothing to write now.
    Queued(PromptAccepted),
}

/// What an abort calls for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AbortDecision {
    /// Nothing runs: the follow-ups (if any) were dropped.
    Idle { dropped: usize },
    /// Interrupt turn `turn`; the follow-ups were dropped.
    Interrupt { turn: u64, dropped: usize },
    /// Turn `turn` is already being interrupted; the follow-ups were
    /// dropped.
    Interrupting { turn: u64, dropped: usize },
}

/// What the reader waits for besides the next event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wait {
    /// Only the next event.
    Event,
    /// The skipped-line grace: a skipped line was the last event.
    Grace,
    /// The interrupted turn's deadline.
    Until(AgentClockInstant),
}

/// What folding one event recorded, and the follow-up it started.
#[derive(Debug, Default)]
pub(crate) struct Folded {
    pub(crate) records: Vec<SessionRecord>,
    pub(crate) step: ProjectionStep,
    pub(crate) follow_up: Option<(u64, String)>,
}

impl SessionCore {
    pub(crate) fn queued(&self) -> usize {
        self.follow_ups.len()
    }

    /// The running turn, if one runs (interrupted or not).
    pub(crate) fn running_turn(&self) -> Option<u64> {
        match self.phase {
            SessionPhase::Busy { turn } | SessionPhase::Interrupting { turn } => Some(turn),
            SessionPhase::NotStarted | SessionPhase::Idle | SessionPhase::Ended => None,
        }
    }

    /// What the reader waits for besides the next event.
    pub(crate) fn wait(&self) -> Wait {
        match (self.phase, self.deadline, self.skipped_last) {
            (SessionPhase::Interrupting { .. }, Some(deadline), _) => Wait::Until(deadline),
            (SessionPhase::Busy { .. }, _, true) => Wait::Grace,
            _ => Wait::Event,
        }
    }

    /// Admit a prompt, as quecto's UDS agent does: idle, it starts a turn;
    /// busy, it needs a behaviour: `Steer` is written into the running
    /// turn, `FollowUp` waits for its end. Nothing is written into a turn
    /// being interrupted.
    pub(crate) fn admit(
        &mut self,
        text: &str,
        behavior: Option<StreamingBehavior>,
    ) -> Result<Admission, SessionRefusal> {
        match (self.phase, behavior) {
            (SessionPhase::NotStarted, _) => Err(SessionRefusal::NotStarted),
            (SessionPhase::Ended, _) => Err(SessionRefusal::Ended),
            (SessionPhase::Idle, _) => Ok(Admission::Write(PromptAccepted::Started {
                turn: self.begin_turn(),
            })),
            (SessionPhase::Busy { .. } | SessionPhase::Interrupting { .. }, None) => {
                Err(SessionRefusal::Busy)
            }
            (SessionPhase::Busy { turn }, Some(StreamingBehavior::Steer)) => {
                Ok(Admission::Write(PromptAccepted::Steered { turn }))
            }
            (SessionPhase::Interrupting { .. }, Some(StreamingBehavior::Steer)) => {
                Err(SessionRefusal::Interrupting)
            }
            (
                SessionPhase::Busy { .. } | SessionPhase::Interrupting { .. },
                Some(StreamingBehavior::FollowUp),
            ) => match self.follow_ups.len() < FOLLOW_UP_QUEUE_CAPACITY {
                true => {
                    self.follow_ups.push_back(text.to_string());
                    Ok(Admission::Queued(PromptAccepted::Queued {
                        position: self.follow_ups.len(),
                    }))
                }
                false => Err(SessionRefusal::QueueFull),
            },
        }
    }

    /// A prompt of the running turn was written under `id`: a result owes
    /// it.
    pub(crate) fn written(&mut self, id: UserTurnId, text: &str) {
        assert!(
            matches!(self.phase, SessionPhase::Busy { .. } | SessionPhase::Ended),
            "only a running turn is written into: {:?}",
            self.phase
        );
        // Ended: the member was closed while the turn was being written.
        if let SessionPhase::Busy { .. } = self.phase {
            self.owed.insert(id);
            self.projector.record_user_turn(text);
        }
    }

    /// A prompt could not be written. A turn it was to start never
    /// started: the member is idle again (its follow-ups wait for the next
    /// turn, or for the member's end).
    pub(crate) fn write_failed(&mut self, accepted: PromptAccepted) {
        if let (PromptAccepted::Started { turn }, SessionPhase::Busy { turn: running }) =
            (accepted, self.phase)
            && turn == running
            && self.owed.is_empty()
        {
            self.phase = SessionPhase::Idle;
        }
    }

    fn begin_turn(&mut self) -> u64 {
        assert_eq!(
            self.phase,
            SessionPhase::Idle,
            "one turn is in flight at a time"
        );
        assert!(self.owed.is_empty(), "an idle member owes nothing");
        let turn = self.last_turn + 1;
        assert!(turn > self.last_turn, "turn ordinals strictly increase");
        self.last_turn = turn;
        self.skipped_last = false;
        self.phase = SessionPhase::Busy { turn };
        turn
    }

    /// The running turn ended: the member is idle, or the next follow-up
    /// starts a turn.
    fn end_turn(&mut self, folded: &mut Folded) {
        assert!(
            self.owed.is_empty(),
            "a turn ends only when nothing is owed"
        );
        self.phase = SessionPhase::Idle;
        self.skipped_last = false;
        self.held = None;
        self.deadline = None;
        if let Some(text) = self.follow_ups.pop_front() {
            let turn = self.begin_turn();
            folded.records.push(SessionRecord::FollowUpStarted {
                turn,
                bytes: text.len(),
            });
            folded.follow_up = Some((turn, text));
        }
    }

    /// Fold one event of the stream.
    pub(crate) fn fold(&mut self, event: &ExternalAgentEvent) -> Folded {
        let turn = self.running_turn();
        let mut folded = Folded::default();
        self.skipped_last = false;
        match event {
            ExternalAgentEvent::AssistantBlock {
                block: AssistantContent::ToolUse { name, .. },
                ..
            } => folded.records.push(SessionRecord::ToolCalled {
                turn,
                tool: recorded_tool_name(name),
            }),
            ExternalAgentEvent::LineSkipped(line) => {
                self.skipped_last = turn.is_some();
                folded.records.push(SessionRecord::LineSkipped {
                    turn,
                    bytes: line.bytes,
                });
            }
            ExternalAgentEvent::Result(result) => {
                match (result.user_turn_ids.as_slice(), self.names_turns) {
                    ([], true) => {}
                    ([], false) => self.owed.clear(),
                    (ids, _) => {
                        self.names_turns = true;
                        for id in ids {
                            self.owed.remove(&UserTurnId(id.clone()));
                        }
                    }
                }
            }
            ExternalAgentEvent::InterruptAnswered(receipt) => {
                for id in &receipt.cancelled {
                    self.owed.remove(&UserTurnId(id.clone()));
                }
            }
            _ => {}
        }
        let mut step = self.projector.apply(event);
        // A result while idle (one no turn of ours owes) is folded into the
        // projection but ends no turn of the session's.
        if let Some(turn) = turn {
            self.settle_turn(turn, event, &mut step, &mut folded);
        }
        folded.step = step;
        folded
    }

    /// After a result or an interrupt's answer: the running turn ends once
    /// nothing is owed, and goes on (its result's end held back) otherwise.
    fn settle_turn(
        &mut self,
        turn: u64,
        event: &ExternalAgentEvent,
        step: &mut ProjectionStep,
        folded: &mut Folded,
    ) {
        let interrupting = matches!(self.phase, SessionPhase::Interrupting { .. });
        match (event, self.owed.is_empty()) {
            (ExternalAgentEvent::Result(_), false) => {
                self.held = step.turn_end.take().or(self.held.take());
                folded.records.push(SessionRecord::TurnContinued {
                    turn,
                    owed: self.owed.len(),
                });
            }
            (ExternalAgentEvent::Result(_), true) => {
                self.end_with(turn, step, folded);
            }
            // Only an interrupt is answered: everything it owed was
            // withdrawn, so no result will come.
            (ExternalAgentEvent::InterruptAnswered(_), true) if interrupting => {
                step.turn_end = self.held.take();
                self.end_with(turn, step, folded);
            }
            _ => {}
        }
    }

    fn end_with(&mut self, turn: u64, step: &ProjectionStep, folded: &mut Folded) {
        folded.records.push(SessionRecord::TurnEnded {
            turn,
            outcome: step
                .turn_end
                .as_ref()
                .map_or("aborted", |outcome| outcome_kind(&outcome.end)),
            duration_ms: step.turn_end.as_ref().and_then(|o| o.duration_ms),
            cost_micro_usd: step.turn_end.as_ref().map_or(0, |o| o.usage.cost_micro_usd),
        });
        self.end_turn(folded);
    }

    /// The running turn to give up, if its last event was a skipped line
    /// (the stream then stayed quiet for the grace).
    pub(crate) fn lost_turn(&self) -> Option<u64> {
        match (self.phase, self.skipped_last) {
            (SessionPhase::Busy { turn }, true) => Some(turn),
            _ => None,
        }
    }

    /// Turn `turn` was interrupted: nothing new is written until every
    /// result it owes has come, or `deadline` passes.
    pub(crate) fn interrupting(&mut self, turn: u64, deadline: AgentClockInstant) {
        assert_eq!(
            self.phase,
            SessionPhase::Busy { turn },
            "only the running turn is interrupted"
        );
        self.phase = SessionPhase::Interrupting { turn };
        self.skipped_last = false;
        self.deadline = Some(deadline);
    }

    /// Whether the interrupted turn is past its deadline at `now`.
    pub(crate) fn interrupt_overdue(&self, now: AgentClockInstant) -> Option<u64> {
        match (self.phase, self.deadline) {
            (SessionPhase::Interrupting { turn }, Some(deadline)) if now >= deadline => Some(turn),
            _ => None,
        }
    }

    /// An abort, as quecto's: its follow-ups are dropped, and a running
    /// turn is interrupted.
    pub(crate) fn abort(&mut self) -> Result<AbortDecision, SessionRefusal> {
        let phase = self.phase;
        let mut drop_follow_ups = || {
            let dropped = self.follow_ups.len();
            self.follow_ups.clear();
            dropped
        };
        match phase {
            SessionPhase::NotStarted => Err(SessionRefusal::NotStarted),
            SessionPhase::Ended => Err(SessionRefusal::Ended),
            SessionPhase::Idle => Ok(AbortDecision::Idle {
                dropped: drop_follow_ups(),
            }),
            SessionPhase::Busy { turn } => Ok(AbortDecision::Interrupt {
                turn,
                dropped: drop_follow_ups(),
            }),
            SessionPhase::Interrupting { turn } => Ok(AbortDecision::Interrupting {
                turn,
                dropped: drop_follow_ups(),
            }),
        }
    }

    /// Hold `step` for the reader.
    pub(crate) fn surface(&mut self, step: SessionStep) {
        self.surfaced.push_back(step);
    }

    pub(crate) fn take_surfaced(&mut self) -> Option<SessionStep> {
        self.surfaced.pop_front()
    }

    /// The member ends: its running turn, if any, and its follow-ups with
    /// it. Returns the turn and how many follow-ups were dropped.
    pub(crate) fn end(&mut self) -> (Option<u64>, usize) {
        let turn = self.running_turn();
        let dropped = self.follow_ups.len();
        self.follow_ups.clear();
        self.owed.clear();
        self.deadline = None;
        self.phase = SessionPhase::Ended;
        (turn, dropped)
    }
}

/// A tool's name as telemetry keeps it: at most
/// [`TOOL_NAME_RECORD_BYTES`], ASCII letters, digits and `_ - . :` only,
/// every other character replaced by `?`.
fn recorded_tool_name(name: &str) -> String {
    let recorded: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' | '.' | ':' => c,
            _ => '?',
        })
        .take(TOOL_NAME_RECORD_BYTES)
        .collect();
    assert!(recorded.len() <= TOOL_NAME_RECORD_BYTES);
    recorded
}

/// A turn end's kind, for telemetry.
fn outcome_kind(end: &TurnEnd) -> &'static str {
    match end {
        TurnEnd::Completed => "completed",
        TurnEnd::Failed(failure) => match failure.kind {
            FailureKind::Aborted => "aborted",
            FailureKind::Error => "failed",
        },
    }
}
