//! The member session's bookkeeping (#2287): its phase, the follow-up
//! queue, the turn ordinals and the projection. Pure: the session use case
//! ([`super::use_cases::DriveExternalAgentSession`]) holds one under its
//! lock and does the I/O each decision calls for.
//!
//! Invariants, asserted: at most one turn is in flight (a turn begins only
//! from `Idle`), turn ordinals strictly increase, and the queue never holds
//! more than [`FOLLOW_UP_QUEUE_CAPACITY`] follow-ups.

use std::collections::VecDeque;

use super::dto::{
    FOLLOW_UP_QUEUE_CAPACITY, ProjectionStep, PromptAccepted, SessionPhase, SessionRecord,
    SessionRefusal, StreamingBehavior,
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
    /// A line of the running turn was skipped: it may have been its
    /// `result`.
    skipped_in_turn: bool,
}

/// What a prompt calls for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Write the prompt to the agent; it is `accepted` once written.
    Write(PromptAccepted),
    /// It was queued: nothing to write now.
    Queued(PromptAccepted),
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

    /// The running turn, if one runs.
    pub(crate) fn running_turn(&self) -> Option<u64> {
        match self.phase {
            SessionPhase::Busy { turn } => Some(turn),
            SessionPhase::NotStarted | SessionPhase::Idle | SessionPhase::Ended => None,
        }
    }

    /// Whether the running turn waits on a stream that skipped a line.
    pub(crate) fn grace_armed(&self) -> bool {
        self.running_turn().is_some() && self.skipped_in_turn
    }

    /// Admit a prompt, as quecto's UDS agent does: idle, it starts a turn;
    /// busy, it needs a behaviour: `Steer` is written into the running
    /// turn, `FollowUp` waits for its end.
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
            (SessionPhase::Busy { .. }, None) => Err(SessionRefusal::Busy),
            (SessionPhase::Busy { turn }, Some(StreamingBehavior::Steer)) => {
                Ok(Admission::Write(PromptAccepted::Steered { turn }))
            }
            (SessionPhase::Busy { .. }, Some(StreamingBehavior::FollowUp)) => {
                match self.follow_ups.len() < FOLLOW_UP_QUEUE_CAPACITY {
                    true => {
                        self.follow_ups.push_back(text.to_string());
                        Ok(Admission::Queued(PromptAccepted::Queued {
                            position: self.follow_ups.len(),
                        }))
                    }
                    false => Err(SessionRefusal::QueueFull),
                }
            }
        }
    }

    /// A prompt could not be written. A turn it was to start never
    /// started: the member is idle again (its follow-ups wait for the next
    /// turn, or for the member's end).
    pub(crate) fn write_failed(&mut self, accepted: PromptAccepted) {
        if let (PromptAccepted::Started { turn }, SessionPhase::Busy { turn: running }) =
            (accepted, self.phase)
            && turn == running
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
        let turn = self.last_turn + 1;
        assert!(turn > self.last_turn, "turn ordinals strictly increase");
        self.last_turn = turn;
        self.skipped_in_turn = false;
        self.phase = SessionPhase::Busy { turn };
        turn
    }

    /// The running turn ended: the member is idle, or the next follow-up
    /// starts a turn.
    fn end_turn(&mut self, folded: &mut Folded) {
        self.phase = SessionPhase::Idle;
        self.skipped_in_turn = false;
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
        match event {
            ExternalAgentEvent::AssistantBlock {
                block: AssistantContent::ToolUse { name, .. },
                ..
            } => folded.records.push(SessionRecord::ToolCalled {
                turn,
                tool: name.clone(),
            }),
            ExternalAgentEvent::LineSkipped(line) => {
                self.skipped_in_turn = turn.is_some();
                folded.records.push(SessionRecord::LineSkipped {
                    turn,
                    bytes: line.bytes,
                });
            }
            _ => {}
        }
        let step = self.projector.apply(event);
        // A result while idle (one a lost turn gave late) is folded into
        // the projection but ends no turn of the session's.
        if let (Some(outcome), Some(turn)) = (&step.turn_end, turn) {
            folded.records.push(SessionRecord::TurnEnded {
                turn,
                outcome: outcome_kind(&outcome.end),
                duration_ms: outcome.duration_ms,
                cost_micro_usd: outcome.usage.cost_micro_usd,
            });
            self.end_turn(&mut folded);
        }
        folded.step = step;
        folded
    }

    /// Give up the running turn, if it skipped a line (the stream then
    /// stayed quiet): the lost turn and what giving it up recorded.
    pub(crate) fn lose_turn(&mut self) -> Option<(u64, Folded)> {
        let turn = self.running_turn().filter(|_| self.skipped_in_turn)?;
        let mut folded = Folded::default();
        folded.records.push(SessionRecord::TurnEnded {
            turn,
            outcome: "lost",
            duration_ms: None,
            cost_micro_usd: 0,
        });
        self.end_turn(&mut folded);
        Some((turn, folded))
    }

    /// The member ends: its running turn, if any, and its follow-ups with
    /// it. Returns the turn and how many follow-ups were dropped.
    pub(crate) fn end(&mut self) -> (Option<u64>, usize) {
        let turn = self.running_turn();
        let dropped = self.follow_ups.len();
        self.follow_ups.clear();
        self.phase = SessionPhase::Ended;
        (turn, dropped)
    }
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
