//! Persist a validated reasoning-effort level as a configured default
//! (#2024 S2): `agents.defaults.effort` in the repository overlay or the
//! global file. The change-reasoning-effort use case is its only caller;
//! composition maps it onto the configuration capability's patch use case
//! (`composition/catalogue_defaults.rs`).

use crate::application::catalogue::ports::{DefaultScope, PersistedDefault};
use crate::domain::provider::EffortLevel;

pub trait EffortDefaultPersistence: Send + Sync {
    /// Record `level` as the default effort of `scope`. Nothing is written
    /// when the adapter refuses; the reason names the remedy.
    fn persist_effort(
        &self,
        scope: DefaultScope,
        level: EffortLevel,
    ) -> Result<PersistedDefault, String>;
}
