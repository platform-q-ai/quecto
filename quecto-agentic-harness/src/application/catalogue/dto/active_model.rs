//! Request and outcome of changing the session's active model (#1847).

use crate::application::catalogue::ports::{DefaultScope, PersistedDefault};
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
    /// Where the model was recorded as a configured default (#2024 S2),
    /// when the switch asked for that.
    pub persisted: Option<PersistedDefault>,
}

/// Why a switch that asked to persist its model as a default did not
/// happen. Nothing changed: the session's model is checked and recorded
/// before it is applied, so a refused record leaves the session as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSwitchError {
    /// The requested id is bare (`model`, not `provider/model`), so there
    /// is no qualified id to record.
    Unqualified { model: String },
    /// The persistence adapter refused or failed; `reason` names the
    /// remedy.
    Persist {
        model: String,
        scope: DefaultScope,
        reason: String,
    },
}

impl std::fmt::Display for ModelSwitchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unqualified { model } => write!(
                f,
                "cannot persist `{model}` as a default: it is a bare id (a later start would \
                 route it to the first configured provider); select it as provider/model"
            ),
            Self::Persist {
                model,
                scope,
                reason,
            } => write!(
                f,
                "model not switched: `{model}` could not be recorded as the {} default: {reason}",
                scope.as_str()
            ),
        }
    }
}

impl std::error::Error for ModelSwitchError {}
