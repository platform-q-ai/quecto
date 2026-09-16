//! Request and outcome of changing the session's active model (#1847).

use crate::domain::catalogue::{ModelRef, UnavailableReason};

/// The per-model limits the loop clamps to, each `None` unless the
/// catalogue declared it explicitly (a synthesized default never clamps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelLimits {
    pub max_output_tokens: Option<u32>,
    pub context_window: Option<usize>,
}

/// What the published runtime generation says about the requested model.
/// The switch proceeds regardless — open-router prefixes accept ids the
/// catalogue cannot enumerate — so this is a verdict, not a gate.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSelectionVerdict {
    /// Known and runnable on `provider` in `generation`.
    Runnable { provider: String, generation: u64 },
    /// The catalogue generation does not know the reference.
    Unknown { reference: String },
    /// Known but not runnable, for these structured reasons.
    NotRunnable {
        reference: ModelRef,
        reasons: Vec<UnavailableReason>,
    },
    /// No runtime has been composed yet (legacy sessions and rigs).
    NoRuntime,
}

/// A planned switch: what the loop will run on and what the catalogue
/// says about it, from one published generation.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSwitchPlan {
    pub model: String,
    pub limits: ModelLimits,
    pub verdict: ModelSelectionVerdict,
}

/// The switch as applied.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSwitched {
    pub plan: ModelSwitchPlan,
    /// Whether the session effort changed as part of the switch.
    pub effort_changed: bool,
}
