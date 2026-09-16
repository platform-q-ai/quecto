//! The agent loop as the catalogue capability's [`EffortRuntime`] (#1067,
//! #1848): the configured effort (`AgentLoopConfig::effort`) is the startup
//! default the use case restores on session switches (admitted for the
//! model then active, so an override never leaks into another session and
//! a startup level never reaches a model that does not accept it); the
//! change-reasoning-effort use case is the only writer of the running level.

use super::AgentLoopImpl;
use crate::application::catalogue::ports::EffortRuntime;
use crate::domain::provider::EffortLevel;

impl EffortRuntime for AgentLoopImpl {
    fn effort(&self) -> Option<EffortLevel> {
        self.effort
    }

    fn startup_effort(&self) -> Option<EffortLevel> {
        self.default_effort
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
}
