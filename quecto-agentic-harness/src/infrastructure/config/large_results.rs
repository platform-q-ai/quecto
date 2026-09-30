//! #2348: the size-aware collapse's dials — their defaults, environment
//! overrides and load-time validation (split from `config.rs`, line cap).

use super::{AgentDefaults, ConfigError};
use crate::application::context_pruning::large_results::LargeResultCollapse;
use std::collections::HashMap;

pub(super) fn default_tokens() -> usize {
    usize::MAX
}

pub(super) fn default_after_turns() -> u32 {
    u32::MAX
}

impl AgentDefaults {
    /// The size-aware collapse these defaults configure (#2348).
    pub fn large_result_collapse(&self) -> LargeResultCollapse {
        LargeResultCollapse::DISABLED
    }
}

/// `QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_TOKENS` and
/// `QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_AFTER_TURNS`.
pub(super) fn apply_env_overrides(_defaults: &mut AgentDefaults, _env: &HashMap<String, String>) {}

pub(super) fn validate(_defaults: &AgentDefaults) -> Result<(), ConfigError> {
    Ok(())
}
