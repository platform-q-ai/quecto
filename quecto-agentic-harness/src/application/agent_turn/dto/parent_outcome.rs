//! What the parent turn-outcome runtime (#2708) hands its transports.
use crate::domain::agents::value_objects::turn_outcome::{
    TurnAdmissionRecord, TurnOutcomeRecord, WorkLiveness,
};
use crate::domain::turn_declaration::TurnIdentity;

/// What opens a guarded outer turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnAdmission {
    /// A human's (or a launcher's) instruction: opens a fresh episode.
    External { episode_id: String },
    /// The one correction the policy asked for.
    Correction,
    /// A note of work the parent owns (a child's or a swarm's).
    WorkWake,
    /// Trusted, consumed-once external child/workflow information.
    FreshWake,
    /// A harness nudge (workflow continuation): never renews the correction.
    Nudge,
}

/// One admitted outer turn. Move-only: it is completed once, and only the
/// runtime's latest admission (by nonce) settles.
#[derive(Debug, PartialEq, Eq)]
pub struct ActiveTurn {
    nonce: u64,
    identity: TurnIdentity,
    episode_id: String,
}

impl ActiveTurn {
    /// The turn the runtime admitted under `nonce`.
    pub fn issued(nonce: u64, identity: TurnIdentity, episode_id: String) -> Self {
        Self {
            nonce,
            identity,
            episode_id,
        }
    }

    /// Its nonce, identity and episode, for the runtime to complete it.
    pub fn into_parts(self) -> (u64, TurnIdentity, String) {
        (self.nonce, self.identity, self.episode_id)
    }

    /// The admission this turn publishes before it starts.
    pub fn admission(&self) -> TurnAdmissionRecord {
        TurnAdmissionRecord {
            session_key: self.identity.session_key.clone(),
            outcome_epoch: self.identity.outcome_epoch.clone(),
            episode_id: self.episode_id.clone(),
            turn_id: self.identity.turn_id.clone(),
        }
    }
}

/// How an admitted turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnEnd {
    /// It completed (and was saved); this is what runs now.
    Completed(WorkLiveness),
    /// Cancelled, failed or aborted: it settles nothing.
    Interrupted,
}

/// What a completed turn comes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParentVerdict {
    /// Nothing to publish: interrupted, or superseded by a newer turn.
    Unsettled,
    /// Run the one correction next.
    Correct,
    /// Publish this outcome.
    Published(TurnOutcomeRecord),
}

/// What a state snapshot carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomeView {
    pub epoch: String,
    pub latest: Option<TurnOutcomeRecord>,
}
