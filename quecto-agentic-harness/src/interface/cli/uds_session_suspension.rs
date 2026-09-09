//! Automatic-turn suspensions dated by swarm control generation (#1721), so
//! a later resume can lift a provider-failure suspension while wakes alone
//! and store rejections never do. Child module: it reaches the session's
//! private fields.
use super::AgentSession;

/// Why a session stopped taking automatic turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspensionCause {
    /// A terminal provider failure; an explicit instruction (prompt, steer,
    /// queued follow-up) or a swarm resume after the failure re-arms it.
    ProviderFailure,
    /// The coordination store durably rejected a wake; only an explicit
    /// instruction re-arms it.
    StoreRejection,
}

/// A suspension and the swarm control generation it was observed under
/// (`None` until the dispatch loop dates it, or when there is no swarm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnSuspension {
    pub cause: SuspensionCause,
    pub generation: Option<u64>,
    /// Whether the generation was read after the failure (not a pre-turn
    /// fallback), so the post-turn dating pass leaves it alone.
    pub dated: bool,
}

impl AgentSession {
    /// Record a control generation this session has seen; never regresses.
    pub(crate) fn observe_control_generation(&mut self, generation: Option<u64>) {
        if let Some(generation) = generation
            && self
                .last_control_generation
                .is_none_or(|seen| generation > seen)
        {
            self.last_control_generation = Some(generation);
        }
    }

    /// Stop automatic turns for `cause`, dated at `generation` when given,
    /// else at the latest control generation this session has seen.
    pub(crate) fn suspend_automatic_turns(
        &mut self,
        cause: SuspensionCause,
        generation: Option<u64>,
    ) {
        self.automatic_turns_allowed = false;
        self.pending_resume_turn = false;
        self.suspension = Some(TurnSuspension {
            cause,
            dated: generation.is_some(),
            generation: generation.or(self.last_control_generation),
        });
    }

    /// An explicit instruction (prompt, steer, admitted follow-up) re-arms
    /// automatic turns.
    pub(crate) fn resume_automatic_turns(&mut self) {
        self.automatic_turns_allowed = true;
        self.pending_resume_turn = false;
        self.suspension = None;
    }

    /// Whether a provider-failure suspension still carries a pre-turn
    /// fallback generation and needs dating from the store.
    pub(crate) fn needs_provider_dating(&self) -> bool {
        self.suspension.is_some_and(|suspension| {
            suspension.cause == SuspensionCause::ProviderFailure && !suspension.dated
        })
    }

    /// Consume the turn owed after a resume (true once per re-arm).
    pub(crate) fn take_pending_resume_turn(&mut self) -> bool {
        std::mem::take(&mut self.pending_resume_turn)
    }

    /// A wake carrying a control generation newer than the one a
    /// provider-failure suspension was observed under means the run was
    /// paused/resumed since: re-arm and report `true`. An undated suspension
    /// takes this generation as its baseline instead (a wake alone must not
    /// clear a suspension). Store rejections never re-arm here.
    pub(crate) fn resume_after_control_change(&mut self, generation: u64) -> bool {
        self.observe_control_generation(Some(generation));
        let Some(suspension) = &mut self.suspension else {
            return false;
        };
        if suspension.cause != SuspensionCause::ProviderFailure {
            return false;
        }
        match suspension.generation {
            Some(seen) if generation > seen => {
                self.resume_automatic_turns();
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
