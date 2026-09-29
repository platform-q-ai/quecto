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
//! A result names at most 64 user turns, so a turn holds at most
//! [`USER_TURNS_PER_TURN_CAPACITY`]. A result naming none, once claude has
//! named them, is a turn of claude's own when it succeeded (it consumed
//! nothing of the member's) and the running turn's end when it failed (a
//! session-scoped failure, a zeroed or a delivery-failure result: none is
//! followed by another result for it). Until claude has named them (an
//! older CLI) a result answers everything written, so no steer is taken:
//! its own result could not be told from the running turn's. claude's
//! `system/init`, which opens its first turn, says sooner that steers may
//! be taken: a CLI at or past the version verified to name them. That is
//! a version's word, not claude's: it admits steers only, and how a
//! result is read is still learnt from one that names its turns (see
//! [`SessionCore::unnamed_result`] for a steer taken on that word). Nor
//! is a steer taken from a CLI whose interrupt does not withdraw the user
//! turns queued behind the running turn (`interrupt_cancel_queued_v1`):
//! one still queued at an abort would survive it and run as a turn nobody
//! bounds.
//!
//! claude's state can become unknown on what it says: the member is then
//! ended ([`Folded::abandon`]) rather than written into while it may be
//! busy, or left waiting for a result that may never come.
//!
//! Invariants, asserted: at most one turn is in flight (a turn begins only
//! from `Idle`), turn ordinals strictly increase, a turn ends only when
//! nothing is owed, only a running turn is interrupted, a turn never holds
//! more than [`USER_TURNS_PER_TURN_CAPACITY`] user turns and the queue
//! never more than [`FOLLOW_UP_QUEUE_CAPACITY`] follow-ups. The member
//! may end (`close`) at any await of the session's: every transition
//! taken after one allows for `Ended`.

use std::collections::{BTreeSet, VecDeque};

use super::dto::{
    AgentClockInstant, FOLLOW_UP_QUEUE_CAPACITY, ProjectionStep, PromptAccepted, SessionPhase,
    SessionRecord, SessionRefusal, SessionStep, StreamingBehavior, TOOL_NAME_RECORD_BYTES,
    TurnOutcome, USER_TURNS_PER_TURN_CAPACITY, UserTurnId,
};
use super::projection::Projector;
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, InterruptReceipt,
};
use crate::domain::external_agent::turn::{FailureKind, TurnEnd};

#[derive(Debug, Default)]
pub(crate) struct SessionCore {
    pub(crate) phase: SessionPhase,
    pub(crate) projector: Projector,
    follow_ups: VecDeque<String>,
    last_turn: u64,
    /// The last event was a skipped line of the running turn (it may have
    /// been its `result`): when the grace for it runs out. Any other event
    /// disarms it.
    skipped_last: Option<AgentClockInstant>,
    /// User turns written and not yet named by a result or withdrawn.
    owed: BTreeSet<UserTurnId>,
    /// User turns written into the running turn: its prompt and steers.
    in_turn: usize,
    /// Whether the agent names the turns its results consumed: learnt
    /// only from a result that does. Until then a result is taken to
    /// answer everything written. Never unlearnt.
    names_turns: bool,
    /// Whether the agent's init says, by its version, that it names them:
    /// steers are admitted on that word before a result has. It decides
    /// nothing about how a result is read. Never unlearnt.
    steerable: bool,
    /// Whether the agent's interrupt withdraws the queued user turns:
    /// learnt from its init's capabilities. Never unlearnt.
    cancels_queued: bool,
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
    /// The skipped-line grace, which runs out at this instant: a skipped
    /// line was the last event.
    Grace(AgentClockInstant),
    /// The interrupted turn's deadline.
    Until(AgentClockInstant),
}

/// What folding one event recorded, and the follow-up it started.
#[derive(Debug, Default)]
pub(crate) struct Folded {
    pub(crate) records: Vec<SessionRecord>,
    pub(crate) step: ProjectionStep,
    pub(crate) follow_up: Option<(u64, String)>,
    /// claude's state became unknown in turn `turn` (it may be busy, or
    /// owe nothing more): the member is to be ended, as when an
    /// interrupted turn never answers. Nothing is written first.
    pub(crate) abandon: Option<u64>,
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
            (SessionPhase::Busy { .. }, _, Some(grace_end)) => Wait::Grace(grace_end),
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
                match (
                    self.names_turns || self.steerable,
                    self.cancels_queued,
                    self.in_turn < USER_TURNS_PER_TURN_CAPACITY,
                ) {
                    (true, true, true) => Ok(Admission::Write(PromptAccepted::Steered { turn })),
                    (true, true, false) => Err(SessionRefusal::QueueFull),
                    (true, false, _) => Err(SessionRefusal::SteerNotWithdrawable),
                    (false, _, _) => Err(SessionRefusal::SteerUnavailable),
                }
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
            self.in_turn += 1;
            assert!(
                self.in_turn <= USER_TURNS_PER_TURN_CAPACITY,
                "a turn holds at most {USER_TURNS_PER_TURN_CAPACITY} user turns"
            );
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
        self.skipped_last = None;
        self.in_turn = 0;
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
        self.skipped_last = None;
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

    /// Fold one event of the stream; a skipped line's grace, if this is
    /// one, runs out at `grace_end`.
    pub(crate) fn fold(
        &mut self,
        event: &ExternalAgentEvent,
        grace_end: AgentClockInstant,
    ) -> Folded {
        let turn = self.running_turn();
        let mut folded = Folded::default();
        self.skipped_last = None;
        match event {
            ExternalAgentEvent::Init(init) => {
                self.steerable |= init.names_turns();
                self.cancels_queued |= init.cancels_queued();
            }
            ExternalAgentEvent::AssistantBlock {
                block: AssistantContent::ToolUse { name, .. },
                ..
            } => folded.records.push(SessionRecord::ToolCalled {
                turn,
                tool: recorded_tool_name(name),
            }),
            ExternalAgentEvent::LineSkipped(line) => {
                self.skipped_last = turn.map(|_| grace_end);
                folded.records.push(SessionRecord::LineSkipped {
                    turn,
                    bytes: line.bytes,
                });
            }
            ExternalAgentEvent::Result(result) => {
                match (result.user_turn_ids.as_slice(), self.names_turns) {
                    ([], true) => self.id_less_result(turn, result.is_error, &mut folded),
                    ([], false) => self.unnamed_result(turn, result.is_error, &mut folded),
                    (ids, _) => {
                        self.names_turns = true;
                        for id in ids {
                            self.owed.remove(&UserTurnId(id.clone()));
                        }
                    }
                }
            }
            // Only an accepted interrupt withdrew anything: a refused one
            // leaves every user turn owed, whatever its answer lists.
            ExternalAgentEvent::InterruptAnswered(receipt) if receipt.accepted => {
                for id in &receipt.cancelled {
                    self.owed.remove(&UserTurnId(id.clone()));
                }
            }
            _ => {}
        }
        let mut step = self.projector.apply(event);
        // A result while idle (one no turn of ours owes) is folded into the
        // projection but ends no turn of the session's.
        match (turn, folded.abandon) {
            (Some(turn), None) => self.settle_turn(turn, event, &mut step, &mut folded),
            // An abandoned turn is not settled: the member ends instead,
            // nothing (no follow-up) is written, and its result reports
            // no end of the turn (the member's end is reported).
            (Some(_), Some(_)) => step.turn_end = None,
            (None, _) => {}
        }
        folded.step = step;
        folded
    }

    /// A result naming no user turn, from a CLI that names them. Only a
    /// success is a turn of claude's own (it consumed nothing of the
    /// member's): any other ends the running turn, as nothing more will
    /// answer what it owes.
    ///
    /// Revisit when members get tools (#2291): a failed turn of claude's
    /// own (a background task's notification that errors) is id-less too,
    /// and this ends the member's running turn early on it. With no tools
    /// claude starts no turn of its own, so none arises today.
    fn id_less_result(&mut self, turn: Option<u64>, is_error: Option<bool>, folded: &mut Folded) {
        let Some(turn) = turn else {
            return;
        };
        let ended = match is_error {
            Some(false) => false,
            Some(true) | None => true,
        };
        if ended {
            self.owed.clear();
        }
        folded
            .records
            .push(SessionRecord::ResultWithoutIds { turn, ended });
    }

    /// A result naming no user turn, before claude has named any: it
    /// answers everything written, and the running turn ends. With no
    /// tools claude runs no turn of its own (#2291), so a CLI whose init
    /// claimed to name turns and whose result names none simply does not
    /// name them; this is that result.
    ///
    /// One case is not safe to read so: a steer was taken on init's word
    /// and the result succeeded. Whether it answered the steer is unknown
    /// (a CLI that names no turns was never verified to fold a steer into
    /// the running turn's result), and were it a turn of claude's own the
    /// member's turns would still be running. Ending the turn could write
    /// the next prompt into a busy claude; holding it could wait forever
    /// for a result already given. So the member is abandoned: nothing
    /// more is written, and no reader waits. A failed result ends the turn
    /// as it does once claude names turns (see [`Self::id_less_result`]).
    fn unnamed_result(&mut self, turn: Option<u64>, is_error: Option<bool>, folded: &mut Folded) {
        let steered = self.in_turn > 1;
        let succeeded = matches!(is_error, Some(false));
        match (turn, steered, succeeded) {
            (Some(turn), true, true) => {
                folded
                    .records
                    .push(SessionRecord::ResultWithoutIds { turn, ended: false });
                folded.abandon = Some(turn);
            }
            (Some(_), true, false) | (Some(_), false, _) | (None, _, _) => self.owed.clear(),
        }
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
            (ExternalAgentEvent::InterruptAnswered(receipt), true)
                if interrupting && receipt.accepted =>
            {
                step.turn_end = self.held.take();
                self.end_with(turn, step, folded);
            }
            // A refused interrupt: claude will not stop a turn that still
            // owes a result, which then runs unbounded. Its state is
            // unknown, so the member ends now rather than at the deadline.
            // Answers carry no request id: a late refusal of an earlier
            // interrupt would end the member too, the safe direction.
            // An accepted one leaves the owed results to come.
            (
                ExternalAgentEvent::InterruptAnswered(InterruptReceipt {
                    accepted: false, ..
                }),
                false,
            ) if interrupting => folded.abandon = Some(turn),
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

    /// The running turn to give up at `now`, if its last event was a
    /// skipped line whose grace has run out (the stream stayed quiet).
    pub(crate) fn lost_turn(&self, now: AgentClockInstant) -> Option<u64> {
        match (self.phase, self.skipped_last) {
            (SessionPhase::Busy { turn }, Some(grace_end)) if now >= grace_end => Some(turn),
            _ => None,
        }
    }

    /// Turn `turn` was interrupted: nothing new is written until every
    /// result it owes has come, or `deadline` passes. `false` when the
    /// member ended (`close`) while the interrupt was being written: there
    /// is nothing left to wait for.
    pub(crate) fn interrupting(&mut self, turn: u64, deadline: AgentClockInstant) -> bool {
        match self.phase {
            SessionPhase::Busy { turn: running } if running == turn => {
                self.phase = SessionPhase::Interrupting { turn };
                self.skipped_last = None;
                self.deadline = Some(deadline);
                true
            }
            SessionPhase::Ended => false,
            phase => panic!("only the running turn {turn} is interrupted: {phase:?}"),
        }
    }

    /// Whether the member has ended.
    pub(crate) fn ended(&self) -> bool {
        self.phase == SessionPhase::Ended
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
