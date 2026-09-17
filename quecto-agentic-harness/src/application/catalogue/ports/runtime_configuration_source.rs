//! The reloadable configuration of one run (#1849): whether the files it
//! was composed from changed, and a fresh provider runtime plus persisted
//! tool policy rebuilt from their current state. Fingerprinting, parsing and
//! composing are infrastructure's; the use case only decides when to
//! rebuild and what to do with the result.

use std::collections::HashMap;
use std::sync::Arc;

use crate::application::providers::ports::LlmProvider;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

/// One rebuilt configuration: everything a reload applies, from one read
/// of the configuration files.
pub struct ReloadedConfiguration {
    /// The provider runtime composed and published from the current files.
    pub provider: Arc<dyn LlmProvider>,
    /// The persisted `tools.policy.entries` baseline, by stable tool id.
    pub tool_policy: HashMap<String, ProfileAvailabilityScope>,
}

pub trait RuntimeConfigurationSource: Send {
    /// Whether any watched file's content changed since it was last
    /// observed. Observing advances the fingerprint: a file that stays as
    /// it is now reports unchanged until it is edited again, even if the
    /// rebuild it prompted fails.
    fn changed(&mut self) -> bool;
    /// Rebuild from the files' current content. The files are observed
    /// before they are read, so a later [`changed`](Self::changed) reports
    /// only edits made after this rebuild — a forced rebuild never prompts
    /// a second one. A failure names why; nothing is published.
    fn rebuild(&mut self) -> Result<ReloadedConfiguration, String>;
}

impl std::fmt::Debug for ReloadedConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReloadedConfiguration")
            .field("provider", &self.provider.name())
            .field("tool_policy", &self.tool_policy)
            .finish()
    }
}
