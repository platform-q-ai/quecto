//! The agent loop as the catalogue capability's [`EffortRuntime`] (#1067,
//! #1848): the configured effort (`AgentLoopConfig::effort`) is the startup
//! default; the change-reasoning-effort use case is the only writer of the
//! running level, and `reset_effort_to_default` restores the startup value
//! on session switches so an override never leaks into another session.

use super::AgentLoopImpl;
use crate::application::catalogue::ports::EffortRuntime;
use crate::domain::provider::EffortLevel;

impl EffortRuntime for AgentLoopImpl {
    fn effort(&self) -> Option<EffortLevel> {
        self.effort
    }

    fn apply_effort(&mut self, level: Option<EffortLevel>) {
        self.effort = level;
    }
}

impl AgentLoopImpl {
    /// The effort level currently applied to every `ChatRequest`
    /// (`None` = provider default).
    pub fn effort(&self) -> Option<EffortLevel> {
        self.effort
    }

    /// Restore the startup (config/provider) default effort. Called on
    /// session switches (`new_session` / `resume_session`) so a runtime
    /// override stays scoped to the session it was set in.
    pub fn reset_effort_to_default(&mut self) {
        self.effort = self.default_effort;
    }
}
