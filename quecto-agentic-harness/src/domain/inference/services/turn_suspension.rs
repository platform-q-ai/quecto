//! When a session stops taking automatic turns, and what re-arms it (#1721,
//! #1909, #2531). A provider failure suspends automatic turns: wakes,
//! queued notes and workflow nudges wait. An explicit instruction re-arms
//! them, and so does a swarm resume after the failure: a control
//! generation newer than the one the suspension was dated at. A failure
//! the provider may recover from (a retryable class whose retries ran out)
//! is re-armed by the next swarm wake or the next automatic note, a bounded
//! number of times, so a recoverable failure never strands a member; a
//! swarm member also wakes itself once after a delay, so the last active
//! member is never left without a wake. A store rejection waits for an
//! explicit instruction alone.
use crate::domain::inference::services::provider_error::classify_provider_error;
use crate::domain::shared::error::DomainError;

/// Recoverable failures the next wakes re-arm before a turn succeeds
/// (#2531); past them a recoverable failure suspends as a terminal one.
pub const MAX_RECOVERABLE_REARMS: u32 = 2;

/// How long a swarm member suspended by a recoverable failure waits before
/// it wakes itself (#2531), so a member no one else wakes still continues.
pub const RECOVERABLE_SELF_WAKE_DELAY: std::time::Duration = std::time::Duration::from_secs(30);

/// The turn a session owes its run once re-armed by a wake, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwedTurn {
    /// A swarm resume after the provider failure re-armed it.
    AfterResume,
    /// A wake re-armed it after a recoverable provider failure (#2531).
    AfterRecoverableFailure,
}

/// Why a session stopped taking automatic turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspensionCause {
    /// A terminal provider failure; an explicit instruction or a swarm
    /// resume after the failure re-arms it.
    ProviderFailure,
    /// A failure of a retryable class whose retries ran out (#2531): the
    /// next swarm wake re-arms it too, within [`MAX_RECOVERABLE_REARMS`].
    RecoverableProviderFailure,
    /// The coordination store durably rejected a wake; only an explicit
    /// instruction re-arms it.
    StoreRejection,
}

impl SuspensionCause {
    /// The cause a turn's failure suspends with: recoverable when its class
    /// is one the provider recovers from (an allowlist: rate limit, server,
    /// network, empty stream, stall), terminal otherwise.
    pub fn for_failure(err: &DomainError) -> Self {
        match classify_provider_error(err).is_retryable() {
            true => Self::RecoverableProviderFailure,
            false => Self::ProviderFailure,
        }
    }

    /// Whether a provider failure caused it (dated, lifted by a resume).
    fn is_provider_failure(self) -> bool {
        matches!(
            self,
            Self::ProviderFailure | Self::RecoverableProviderFailure
        )
    }
}

/// What a swarm wake did to a session's automatic turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeOutcome {
    /// Nothing was suspended.
    NotSuspended,
    /// Still suspended: no resume since the failure, or a store rejection.
    StillSuspended,
    /// A resume since the failure re-armed it; the run is owed a turn.
    RearmedByResume,
    /// The wake re-armed a recoverable failure (#2531); the run is owed a
    /// turn to continue the interrupted work.
    RearmedAfterRecoverableFailure,
}

/// What a suspended session saw when a wake made it read its run's status:
/// the wake's outcome and the control generation read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuspendedProbe {
    pub outcome: WakeOutcome,
    pub generation: u64,
}

/// A suspension and the swarm control generation it was observed under
/// (`None` until it is dated, or when there is no swarm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnSuspension {
    pub cause: SuspensionCause,
    pub generation: Option<u64>,
    /// Whether the generation was read after the failure (not a pre-turn
    /// fallback), so a later dating pass leaves it alone.
    pub dated: bool,
}

/// The explicit instructions that re-arm a suspended session: an
/// allowlist. Automatic work (wakes, notes, nudges) is not an instruction
/// and has no variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// A client's prompt.
    Prompt,
    /// A client's steer.
    Steer,
    /// A queued follow-up or control the run admitted.
    FollowUp,
}

impl Instruction {
    /// A client's instruction: its steer, or its prompt.
    pub fn client(steer: bool) -> Self {
        match steer {
            true => Self::Steer,
            false => Self::Prompt,
        }
    }
}

/// The work that would start a turn (#1909): an explicit instruction, or
/// automatic work (a swarm wake, a queued note, a workflow nudge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Instruction(Instruction),
    Automatic,
}

impl Work {
    /// A queued follow-up instruction.
    pub const FOLLOW_UP: Self = Self::Instruction(Instruction::FollowUp);

    /// Whether this is a client's steer, which goes ahead of queued work.
    pub fn is_steer(self) -> bool {
        matches!(self, Self::Instruction(Instruction::Steer))
    }
}

/// Whether a session takes automatic turns, the suspension that stopped
/// them, the latest control generation it has seen, and the turn a resume
/// owes the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomaticTurns {
    allowed: bool,
    suspension: Option<TurnSuspension>,
    last_control_generation: Option<u64>,
    owed_turn: Option<OwedTurn>,
    /// Recoverable failures re-armed since a turn last succeeded.
    recoverable_rearms: u32,
}

impl Default for AutomaticTurns {
    fn default() -> Self {
        Self {
            allowed: true,
            suspension: None,
            last_control_generation: None,
            owed_turn: None,
            recoverable_rearms: 0,
        }
    }
}

impl AutomaticTurns {
    /// Whether automatic turns may run.
    pub fn allowed(&self) -> bool {
        self.allowed
    }

    /// The suspension in force, if any.
    pub fn suspension(&self) -> Option<TurnSuspension> {
        self.suspension
    }

    /// Record a control generation this session has seen; never regresses.
    pub fn observe_control_generation(&mut self, generation: Option<u64>) {
        if let Some(generation) = generation
            && self
                .last_control_generation
                .is_none_or(|seen| generation > seen)
        {
            self.last_control_generation = Some(generation);
        }
    }

    /// Stop automatic turns for `cause`, dated at `generation` when given,
    /// else at the latest control generation this session has seen. Any
    /// turn a resume owed is cancelled.
    pub fn suspend(&mut self, cause: SuspensionCause, generation: Option<u64>) {
        self.allowed = false;
        self.owed_turn = None;
        self.suspension = Some(TurnSuspension {
            cause,
            dated: generation.is_some(),
            generation: generation.or(self.last_control_generation),
        });
        debug_assert!(!self.allowed && self.suspension.is_some());
    }

    /// Whether `work` may start a turn now: an instruction always (it
    /// re-arms a suspended session); automatic work while automatic turns
    /// run, or to re-arm a recoverable failure (#2531).
    pub fn admits(&self, work: Work) -> bool {
        match work {
            Work::Instruction(_) => true,
            Work::Automatic => self.allowed || self.recoverable(),
        }
    }

    /// Whether a recoverable provider failure is what suspended the session.
    fn recoverable(&self) -> bool {
        self.suspension.is_some_and(|suspension| {
            suspension.cause == SuspensionCause::RecoverableProviderFailure
        })
    }

    /// Whether this session should wake itself after
    /// [`RECOVERABLE_SELF_WAKE_DELAY`] (#2531): a recoverable failure
    /// suspended it, so nothing else may wake it again.
    pub fn needs_self_wake(&self) -> bool {
        self.recoverable()
    }

    /// `work` starts a turn. A client's prompt or steer re-arms automatic
    /// turns and does the work a wake's turn was owed for; an admitted
    /// follow-up re-arms a suspended session and leaves an owed turn to a
    /// running one; automatic work runs while turns run, re-arms a
    /// recoverable failure (it is itself the turn, #2531), and never
    /// starts a session suspended any other way.
    pub fn start(&mut self, work: Work) {
        match work {
            Work::Instruction(Instruction::Prompt | Instruction::Steer) => self.rearm(),
            Work::Instruction(Instruction::FollowUp) => match self.allowed {
                true => {}
                false => self.rearm(),
            },
            Work::Automatic => match (self.allowed, self.recoverable()) {
                (true, _) => {}
                // The automatic work is itself the turn: none is owed.
                (false, true) => self.rearm_recoverable(),
                (false, false) => panic!("automatic work starts only once admitted"),
            },
        }
        debug_assert!(self.allowed);
    }

    fn rearm(&mut self) {
        self.allowed = true;
        self.owed_turn = None;
        self.suspension = None;
        self.recoverable_rearms = 0;
    }

    /// Re-arm a recoverable failure (#2531), counted against the bound.
    fn rearm_recoverable(&mut self) {
        debug_assert!(self.recoverable());
        debug_assert!(self.recoverable_rearms < MAX_RECOVERABLE_REARMS);
        self.allowed = true;
        self.suspension = None;
        self.recoverable_rearms += 1;
    }

    /// A turn's failure suspends automatic turns: recoverable while the
    /// bound lasts ([`SuspensionCause::for_failure`]), terminal after it.
    pub fn suspend_after_failure(&mut self, err: &DomainError, generation: Option<u64>) {
        let within_bound = self.recoverable_rearms < MAX_RECOVERABLE_REARMS;
        let cause = match (SuspensionCause::for_failure(err), within_bound) {
            (SuspensionCause::RecoverableProviderFailure, true) => {
                SuspensionCause::RecoverableProviderFailure
            }
            (SuspensionCause::RecoverableProviderFailure, false)
            | (SuspensionCause::ProviderFailure, _) => SuspensionCause::ProviderFailure,
            (SuspensionCause::StoreRejection, _) => {
                unreachable!("a turn's failure is never a store rejection")
            }
        };
        self.suspend(cause, generation);
    }

    /// A turn succeeded: the recoverable bound is whole again.
    pub fn turn_succeeded(&mut self) {
        self.recoverable_rearms = 0;
    }

    /// A swarm wake at control `generation`: a resume since a provider
    /// failure re-arms it and owes the run a turn; otherwise a recoverable
    /// failure is re-armed by the wake (#2531), within its bound, and owes
    /// the run a turn to continue the interrupted work, whether or not the
    /// wake itself brings work.
    pub fn wake(&mut self, generation: u64) -> WakeOutcome {
        if self.resume_after_control_change(generation) {
            return WakeOutcome::RearmedByResume;
        }
        match self.suspension.map(|suspension| suspension.cause) {
            None => WakeOutcome::NotSuspended,
            Some(SuspensionCause::RecoverableProviderFailure) => {
                self.rearm_recoverable();
                self.owed_turn = Some(OwedTurn::AfterRecoverableFailure);
                WakeOutcome::RearmedAfterRecoverableFailure
            }
            Some(SuspensionCause::ProviderFailure | SuspensionCause::StoreRejection) => {
                WakeOutcome::StillSuspended
            }
        }
    }

    /// Whether a provider-failure suspension still carries a pre-turn
    /// fallback generation and needs dating.
    pub fn needs_provider_dating(&self) -> bool {
        self.suspension
            .is_some_and(|suspension| suspension.cause.is_provider_failure() && !suspension.dated)
    }

    /// Date a provider-failure suspension at `generation`, the control
    /// generation read after the failure (#1721); its cause stays.
    pub fn date_suspension(&mut self, generation: u64) {
        if let Some(suspension) = self
            .suspension
            .as_mut()
            .filter(|suspension| suspension.cause.is_provider_failure())
        {
            suspension.generation = Some(generation);
            suspension.dated = true;
        }
    }

    /// Consume the turn owed after a wake re-armed the session (once per
    /// re-arm), and why it is owed.
    pub fn take_owed_turn(&mut self) -> Option<OwedTurn> {
        self.owed_turn.take()
    }

    /// A control generation newer than the one a provider-failure
    /// suspension was observed under means the run was paused and resumed
    /// since: re-arm, owe the run a turn, and report `true`. An undated
    /// suspension takes this generation as its baseline instead (a wake
    /// alone never clears a suspension). Store rejections never re-arm here.
    fn resume_after_control_change(&mut self, generation: u64) -> bool {
        self.observe_control_generation(Some(generation));
        let Some(suspension) = &mut self.suspension else {
            return false;
        };
        // Only a provider-failure suspension lifts on a resume.
        match suspension.cause {
            SuspensionCause::ProviderFailure | SuspensionCause::RecoverableProviderFailure => {}
            SuspensionCause::StoreRejection => return false,
        }
        match suspension.generation {
            Some(seen) if generation > seen => {
                self.allowed = true;
                self.suspension = None;
                self.owed_turn = Some(OwedTurn::AfterResume);
                true
            }
            Some(_) => false,
            None => {
                suspension.generation = Some(generation);
                false
            }
        }
    }
}

#[cfg(test)]
#[path = "turn_suspension_tests.rs"]
mod tests;
