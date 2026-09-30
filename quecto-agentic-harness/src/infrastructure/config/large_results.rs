//! #2348: the size-aware collapse's dials — their defaults, environment
//! overrides and load-time validation (split from `config.rs`, line cap).

use super::{AgentDefaults, ConfigError};
use crate::domain::large_result_collapse::LargeResultCollapse;
use std::collections::HashMap;

/// A tool result over this many estimated tokens is "large". Chosen from
/// the owner's three swarm sessions (#2348): 2k marks the top 3-6% of
/// results (5, 5 and 2 of 108, 78 and 50) yet carries most of what is
/// re-sent: at the shipped tool dial of 50 the rule saves 25%, 17% and
/// 33% of their simulated input. 1k would save 37-40% but stubs the
/// 1-2k reads a worker edits against and doubles the prefix rewrites
/// (21 against 12 for the run 2 coordinator); 4k saves only 10-33%.
const SWARM_DEFAULT_TOKENS: usize = 2_000;

/// How many model responses see a large result in full before it is
/// stubbed. At 3 the model has read it and acted on it twice more; 2 and 5
/// differ from 3 by about one point on the same sessions.
const DEFAULT_AFTER_TURNS: u32 = 3;

impl AgentDefaults {
    /// The size-aware collapse for an agent that takes part in no swarm
    /// (#2348 review M1): off unless `context_collapse_large_result_tokens`
    /// is set.
    pub fn large_result_collapse(&self) -> LargeResultCollapse {
        self.swarm_large_result_collapse()
    }

    /// The size-aware collapse once the process takes part in a swarm:
    /// 2000 tokens seen for 3 turns unless configured.
    pub fn swarm_large_result_collapse(&self) -> LargeResultCollapse {
        LargeResultCollapse {
            over_tokens: self
                .context_collapse_large_result_tokens
                .unwrap_or(SWARM_DEFAULT_TOKENS),
            after_turns: self
                .context_collapse_large_result_after_turns
                .unwrap_or(DEFAULT_AFTER_TURNS),
        }
    }
}

/// `QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_TOKENS` and
/// `QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_AFTER_TURNS`; a value that is no
/// count is ignored, as the other numeric overrides ignore it.
pub(super) fn apply_env_overrides(defaults: &mut AgentDefaults, env: &HashMap<String, String>) {
    if let Some(n) = env
        .get("QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_TOKENS")
        .and_then(|v| v.parse::<usize>().ok())
    {
        defaults.context_collapse_large_result_tokens = Some(n);
    }
    if let Some(n) = env
        .get("QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_AFTER_TURNS")
        .and_then(|v| v.parse::<u32>().ok())
    {
        defaults.context_collapse_large_result_after_turns = Some(n);
    }
}

/// 0 turns would stub a result the model has not seen (#2213): refused.
/// The rule is switched off with a size no result reaches instead.
pub(super) fn validate(defaults: &AgentDefaults) -> Result<(), ConfigError> {
    match defaults.context_collapse_large_result_after_turns {
        None | Some(1..) => Ok(()),
        Some(0) => Err(ConfigError::ContextBudget(
            "context_collapse_large_result_after_turns must be at least 1: a result \
             the model has not seen is never collapsed; set \
             context_collapse_large_result_tokens to 18446744073709551615 to switch \
             the rule off"
                .to_string(),
        )),
    }
}
