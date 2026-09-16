//! The session state effort changes act on (#1848): the level the agent
//! loop applies to every subsequent `ChatRequest`. The loop implements it;
//! the use case is the only writer.

use crate::domain::provider::EffortLevel;

pub trait EffortRuntime {
    /// The level currently applied (`None` = provider default).
    fn effort(&self) -> Option<EffortLevel>;
    /// The level the run started with (config or flag), as admitted for
    /// the startup model; `None` = provider default.
    fn startup_effort(&self) -> Option<EffortLevel>;
    /// Apply `level` from the next turn on.
    fn apply_effort(&mut self, level: Option<EffortLevel>);
}
