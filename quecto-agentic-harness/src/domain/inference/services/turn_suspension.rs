//! When a session stops taking automatic turns, and what re-arms it (#1721,
//! #1909). A terminal provider failure suspends automatic turns: wakes,
//! queued notes and workflow nudges wait. An explicit instruction re-arms
//! them, and so does a swarm resume after the failure: a control
//! generation newer than the one the suspension was dated at. A store
//! rejection waits for an explicit instruction alone.

/// Why a session stopped taking automatic turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspensionCause {
    /// A terminal provider failure; an explicit instruction or a swarm
    /// resume after the failure re-arms it.
    ProviderFailure,
    /// The coordination store durably rejected a wake; only an explicit
    /// instruction re-arms it.
    StoreRejection,
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

/// Whether a session takes automatic turns, the suspension that stopped
/// them, the latest control generation it has seen, and the turn a resume
/// owes the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomaticTurns {
    allowed: bool,
    suspension: Option<TurnSuspension>,
    last_control_generation: Option<u64>,
    pending_resume_turn: bool,
}

impl Default for AutomaticTurns {
    fn default() -> Self {
        Self {
            allowed: true,
            suspension: None,
            last_control_generation: None,
            pending_resume_turn: false,
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
        self.pending_resume_turn = false;
        self.suspension = Some(TurnSuspension {
            cause,
            dated: generation.is_some(),
            generation: generation.or(self.last_control_generation),
        });
        debug_assert!(!self.allowed && self.suspension.is_some());
    }

    /// An explicit instruction re-arms automatic turns and does the work a
    /// resume turn was owed for.
    pub fn rearm(&mut self, _instruction: Instruction) {
        self.allowed = true;
        self.pending_resume_turn = false;
        self.suspension = None;
    }

    /// Whether a provider-failure suspension still carries a pre-turn
    /// fallback generation and needs dating.
    pub fn needs_provider_dating(&self) -> bool {
        self.suspension.is_some_and(|suspension| {
            suspension.cause == SuspensionCause::ProviderFailure && !suspension.dated
        })
    }

    /// Consume the turn owed after a resume (true once per re-arm).
    pub fn take_pending_resume_turn(&mut self) -> bool {
        std::mem::take(&mut self.pending_resume_turn)
    }

    /// A control generation newer than the one a provider-failure
    /// suspension was observed under means the run was paused and resumed
    /// since: re-arm, owe the run a turn, and report `true`. An undated
    /// suspension takes this generation as its baseline instead (a wake
    /// alone never clears a suspension). Store rejections never re-arm here.
    pub fn resume_after_control_change(&mut self, generation: u64) -> bool {
        self.observe_control_generation(Some(generation));
        let Some(suspension) = &mut self.suspension else {
            return false;
        };
        if suspension.cause != SuspensionCause::ProviderFailure {
            return false;
        }
        match suspension.generation {
            Some(seen) if generation > seen => {
                self.allowed = true;
                self.suspension = None;
                self.pending_resume_turn = true;
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
