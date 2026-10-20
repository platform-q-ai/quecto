//! Resume work after a provider failure (#1909, #2531): a session's
//! automatic turns stop when a turn fails, and are re-armed by an explicit
//! instruction, by a swarm resume after the failure, or, for a failure the
//! provider may recover from, by the next swarm wake (bounded).
//!
//! The rule is the domain's (`turn_suspension`); this use case holds the
//! session's state and runs it. The swarm's run control is the one effect
//! it needs, to date a suspension and to see a resume: the delivery
//! adapter owns that port for the process and passes it in.
use crate::application::swarm::ports::lifecycle::SwarmRunControl;
use crate::domain::inference::services::turn_suspension::{
    AutomaticTurns, SuspendedProbe, SuspensionCause, Work,
};
use crate::domain::shared::error::DomainError;
use crate::domain::swarm::value_objects::run_control::RunControlAction;

/// One session's automatic turns: whether they run, why not, and what
/// re-arms them.
#[derive(Debug, Default)]
pub struct ResumeAfterProviderFailure {
    turns: AutomaticTurns,
}

impl ResumeAfterProviderFailure {
    /// A session taking automatic turns, nothing suspended.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether automatic turns may run.
    pub fn allowed(&self) -> bool {
        self.turns.allowed()
    }

    /// Record a swarm control generation the session has seen.
    pub fn observe_control_generation(&mut self, generation: Option<u64>) {
        self.turns.observe_control_generation(generation);
    }

    /// A turn ended in `err`: suspend, recoverable or terminal by its class
    /// (#2531), dated later ([`Self::date_suspension`]) at the generation
    /// known after it.
    pub fn suspend_after_failure(&mut self, err: &DomainError) {
        self.turns.suspend_after_failure(err, None);
        debug_assert!(!self.turns.allowed());
    }

    /// A turn succeeded: the recoverable re-arm bound is whole again.
    pub fn turn_succeeded(&mut self) {
        self.turns.turn_succeeded();
    }

    /// The coordination store durably rejected a wake: suspend until an
    /// explicit instruction.
    pub fn suspend_after_store_rejection(&mut self) {
        self.suspend(SuspensionCause::StoreRejection, None);
    }

    /// Stop automatic turns for `cause`, dated at `generation` when it is
    /// known, else at the latest control generation the session has seen.
    fn suspend(&mut self, cause: SuspensionCause, generation: Option<u64>) {
        self.turns.suspend(cause, generation);
        debug_assert!(!self.turns.allowed());
    }

    /// Whether `work` may start a turn now (the domain decides: an
    /// instruction always, automatic work only while automatic turns run).
    pub fn admits(&self, work: Work) -> bool {
        self.turns.admits(work)
    }

    /// `work` starts a turn ([`AutomaticTurns::start`]); a failure during
    /// it is dated at `generation`, the control generation known now
    /// (#1721).
    pub fn start_turn(&mut self, work: Work, generation: Option<u64>) {
        self.turns.start(work);
        self.observe_control_generation(generation);
    }

    /// Consume the turn a resume owed the run (true once per re-arm).
    pub fn take_owed_turn(&mut self) -> bool {
        self.turns.take_pending_resume_turn()
    }

    /// Date a provider-failure suspension at the control generation current
    /// after the failure (#1721), so a pause and resume during the failed
    /// turn cannot re-arm it; only a later resume can. Without a swarm, or
    /// when the store cannot answer, the pre-turn generation stands. Only a
    /// suspension not yet dated this way is touched. Returns the generation
    /// read, for the caller to share with the rest of the process.
    pub async fn date_suspension(&mut self, control: Option<&dyn SwarmRunControl>) -> Option<u64> {
        let control = control.filter(|_| self.turns.needs_provider_dating())?;
        let receipt = control.apply(RunControlAction::Status).await.ok()?;
        self.turns.date_suspension(receipt.generation);
        debug_assert!(!self.turns.needs_provider_dating());
        Some(receipt.generation)
    }

    /// A suspended session got a wake: learn the run's control generation
    /// without claiming the wake. A resume since the provider failure
    /// re-arms automatic turns and owes the run a turn; a recoverable
    /// failure is re-armed by the wake itself (#2531); otherwise a wake
    /// alone never re-arms.
    pub async fn probe_while_suspended(
        &mut self,
        control: &dyn SwarmRunControl,
    ) -> Result<SuspendedProbe, DomainError> {
        debug_assert!(!self.turns.allowed(), "only a suspended session probes");
        let receipt = control.apply(RunControlAction::Status).await?;
        let generation = receipt.generation;
        Ok(SuspendedProbe {
            outcome: self.turns.wake(generation),
            generation,
        })
    }
}

#[cfg(test)]
#[path = "resume_after_provider_failure_tests.rs"]
mod tests;
