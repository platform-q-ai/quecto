//! #1067: runtime reasoning-effort mutation for the agent loop.
//!
//! The configured effort (`AgentLoopConfig::effort`) is the startup default;
//! `set_effort` overrides it for the running session (every subsequent
//! `ChatRequest` carries the new level) and `reset_effort_to_default`
//! restores the startup value on session switches so an override never
//! leaks into another session.

use super::AgentLoopImpl;
use crate::domain::provider::EffortLevel;

impl AgentLoopImpl {
    /// The effort level currently applied to every `ChatRequest`
    /// (`None` = provider default).
    pub fn effort(&self) -> Option<EffortLevel> {
        self.effort
    }

    /// Override the session's effort level; applies from the next turn.
    pub fn set_effort(&mut self, effort: EffortLevel) {
        self.effort = Some(effort);
    }

    /// Restore the startup (config/provider) default effort. Called on
    /// session switches (`new_session` / `resume_session`) so a runtime
    /// override stays scoped to the session it was set in.
    pub fn reset_effort_to_default(&mut self) {
        self.effort = self.default_effort;
    }
}
