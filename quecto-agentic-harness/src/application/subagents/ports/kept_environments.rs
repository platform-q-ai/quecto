//! The environments a selected termination kept (#2681): an owner who kills
//! a member whose teardown keeps its environment (#1924, #2666) is told so
//! in the kill's own answer, with the environment's ref and the reason its
//! teardown recorded — the record's own text, never a second wording.

use crate::domain::identity::ids::AgentUuid;

/// An environment a teardown kept rather than ended (#2681): its ref and
/// the reason its record holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptEnvironment {
    pub environment_ref: String,
    pub reason: String,
}

/// What the session's environments say of a teardown's agents.
pub trait KeptEnvironments: Send + Sync {
    /// The refs of the live (not yet retained) environments any of
    /// `agents` is a member of, read before their teardown, each once.
    fn memberships(&self, agents: &[AgentUuid]) -> Vec<String>;

    /// Of `environments`, each now retained, with the reason its teardown
    /// recorded, in the order given. One no longer known, or not retained,
    /// is left out.
    fn kept(&self, environments: &[String]) -> Vec<KeptEnvironment>;
}
