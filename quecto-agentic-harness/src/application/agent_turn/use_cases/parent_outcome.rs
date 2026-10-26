//! The turn-outcome guard of a trusted top-level parent (#2708): one
//! runtime both transports drive. A transport admits each outer turn it is
//! about to run, completes it with what it observed, and publishes what
//! this use case decides; the policy is the domain's.
use std::sync::{Mutex, MutexGuard};

pub use crate::application::agent_turn::dto::parent_outcome::{
    ActiveTurn, OutcomeView, ParentVerdict, TurnAdmission, TurnEnd,
};
use crate::application::agent_turn::use_cases::declaration::TurnDeclarationService;
use crate::domain::agents::services::turn_outcome_policy::{TurnVerdict, settle_turn};
use crate::domain::agents::value_objects::turn_outcome::{TurnOutcomeRecord, WorkLiveness};
use crate::domain::shared::error::DomainError;
use crate::domain::turn_declaration::{
    LaunchAuthority, TurnBoundary, TurnDeclaration, TurnIdentity, TurnIntent,
};

/// Whether this episode's one correction is still to come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Correction {
    Unspent,
    /// The policy asked for it: the next admission may be the correction.
    Owed,
    Spent,
}

struct RuntimeState {
    epoch: String,
    /// The external episode the current turns belong to; none before the
    /// first instruction of an epoch.
    episode: Option<String>,
    correction: Correction,
    /// A terminal outcome was published: no automatic turn until a fresh
    /// external instruction.
    fenced: bool,
    sequence: u64,
    latest: Option<TurnOutcomeRecord>,
    /// The admitted turn, by nonce; a newer admission supersedes it.
    active: Option<(u64, TurnIdentity)>,
    next_nonce: u64,
}

pub struct ParentOutcomeRuntime {
    state: Mutex<RuntimeState>,
    declarations: TurnDeclarationService,
    mint: fn() -> String,
}

impl ParentOutcomeRuntime {
    /// A runtime whose epochs `mint` names, one fresh name each.
    pub fn new(mint: fn() -> String) -> Self {
        Self {
            state: Mutex::new(RuntimeState {
                epoch: mint(),
                episode: None,
                correction: Correction::Unspent,
                fenced: false,
                sequence: 0,
                latest: None,
                active: None,
                next_nonce: 0,
            }),
            declarations: TurnDeclarationService::new(Self::no_turn()),
            mint,
        }
    }

    /// The identity no admitted turn has: what the declarations hold
    /// between turns, so nothing declares outside one.
    fn no_turn() -> TurnIdentity {
        TurnIdentity {
            session_key: String::new(),
            outcome_epoch: String::new(),
            turn_id: String::new(),
        }
    }

    fn state(&self) -> MutexGuard<'_, RuntimeState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether an automatic turn (a note, a wake, a nudge) may run now:
    /// never after a terminal outcome until a fresh external instruction.
    pub fn admits_automatic(&self) -> bool {
        !self.state().fenced
    }

    /// Admit the outer turn whose committed opener is `turn_id`. An
    /// external instruction always opens a fresh episode; a correction only
    /// when the policy owes one; an automatic turn only inside an unfenced
    /// episode (`None` otherwise: the caller runs it unguarded or not at all).
    pub fn admit(
        &self,
        session_key: &str,
        admission: TurnAdmission,
        turn_id: &str,
    ) -> Option<ActiveTurn> {
        let mut state = self.state();
        let open = state.episode.is_some() && !state.fenced;
        match (admission, open, state.correction) {
            (TurnAdmission::External { episode_id }, _, _) => {
                state.episode = Some(episode_id);
                state.fenced = false;
                state.correction = Correction::Unspent;
            }
            (TurnAdmission::Correction, true, Correction::Owed) => {
                state.correction = Correction::Spent;
            }
            (TurnAdmission::WorkWake, true, _) => state.correction = Correction::Unspent,
            (TurnAdmission::Nudge, true, _) => {}
            (TurnAdmission::Correction | TurnAdmission::WorkWake | TurnAdmission::Nudge, _, _) => {
                return None;
            }
        }
        let nonce = state
            .next_nonce
            .checked_add(1)
            .expect("turn nonces never wrap");
        state.next_nonce = nonce;
        let identity = TurnIdentity {
            session_key: session_key.to_owned(),
            outcome_epoch: state.epoch.clone(),
            turn_id: turn_id.to_owned(),
        };
        let episode_id = state
            .episode
            .clone()
            .expect("an admitted turn has an episode");
        state.active = Some((nonce, identity.clone()));
        // Lock order: runtime state, then declarations.
        let entered = self.declarations.enter(TurnBoundary::OuterRun, &identity);
        debug_assert!(entered.is_ok(), "an outer run always enters");
        Some(ActiveTurn::issued(nonce, identity, episode_id))
    }

    /// Complete an admitted turn with what its transport observed once it
    /// was saved. Only the latest admission settles; it is released either way.
    pub fn complete(&self, turn: ActiveTurn, end: TurnEnd) -> ParentVerdict {
        let (nonce, identity, episode_id) = turn.into_parts();
        let mut state = self.state();
        if state.active.as_ref().map(|(active, _)| *active) != Some(nonce) {
            return ParentVerdict::Unsettled;
        }
        state.active = None;
        let declared = self.declarations.snapshot();
        let _ = self
            .declarations
            .enter(TurnBoundary::OuterRun, &Self::no_turn());
        let TurnEnd::Completed(liveness) = end else {
            return ParentVerdict::Unsettled;
        };
        // An unreadable declaration is never read as "nothing declared".
        let (declaration, liveness) = match declared {
            Ok(declaration) => (
                declaration.filter(|declared| declared.identity == identity),
                liveness,
            ),
            Err(_) => (None, liveness.and(WorkLiveness::Unknown)),
        };
        // A correction owed but never run counts as spent: never two.
        let spent = matches!(state.correction, Correction::Owed | Correction::Spent);
        match settle_turn(declaration.as_ref(), liveness, spent) {
            TurnVerdict::Correct => {
                debug_assert_eq!(state.correction, Correction::Unspent);
                state.correction = Correction::Owed;
                ParentVerdict::Correct
            }
            TurnVerdict::Settle(outcome, reason) => {
                state.sequence = state.sequence.checked_add(1).expect("sequences never wrap");
                let record = TurnOutcomeRecord {
                    session_key: identity.session_key,
                    outcome_epoch: identity.outcome_epoch,
                    episode_id,
                    turn_id: identity.turn_id,
                    sequence: state.sequence,
                    outcome,
                    reason,
                };
                debug_assert_eq!(record.outcome_epoch, state.epoch);
                state.fenced = outcome.is_terminal();
                state.latest = Some(record.clone());
                ParentVerdict::Published(record)
            }
        }
    }

    /// The conversation's history was replaced (a new session, a resume,
    /// a clear, a rewind): a fresh epoch with no episode and no outcome. The
    /// sequence keeps advancing.
    pub fn history_replaced(&self) {
        let mut state = self.state();
        state.epoch = (self.mint)();
        state.episode = None;
        state.correction = Correction::Unspent;
        state.fenced = false;
        state.latest = None;
        state.active = None;
        let _ = self
            .declarations
            .enter(TurnBoundary::OuterRun, &Self::no_turn());
    }

    /// The epoch and latest outcome a state snapshot carries.
    pub fn view(&self) -> OutcomeView {
        let state = self.state();
        OutcomeView {
            epoch: state.epoch.clone(),
            latest: state.latest.clone(),
        }
    }
}

impl std::fmt::Debug for ParentOutcomeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParentOutcomeRuntime")
            .finish_non_exhaustive()
    }
}

impl crate::application::agent_turn::ports::DeclareTurnOutcome for ParentOutcomeRuntime {
    fn declare(
        &self,
        authority: LaunchAuthority,
        identity: &TurnIdentity,
        intent: TurnIntent,
        reason: &str,
    ) -> Result<TurnDeclaration, DomainError> {
        self.declarations
            .declare(authority, identity, intent, reason)
    }
}

impl crate::application::agent_turn::ports::BoundTurnDeclaration for ParentOutcomeRuntime {
    fn capture(&self) -> Option<TurnIdentity> {
        self.state()
            .active
            .as_ref()
            .map(|(_, identity)| identity.clone())
    }
}

#[cfg(test)]
#[path = "parent_outcome_tests.rs"]
mod issue_2708_parent_runtime_tests;
