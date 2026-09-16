//! Request and outcome of changing the session's reasoning effort (#1848,
//! #1996).

use crate::domain::provider::EffortLevel;

/// Change the effort applied to subsequent turns of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortChangeRequest {
    /// The session's active model, as a qualified `provider/model` id.
    pub model: String,
    /// The requested level, as typed on the wire.
    pub level: String,
}

/// The level now in effect and the vocabulary it was chosen from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortChangeOutcome {
    pub effective: EffortLevel,
    pub vocabulary: Vec<EffortLevel>,
}

/// Why a requested level was refused. Every variant names what the model
/// does accept, so the user sees only choices the wire will honour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffortChangeError {
    /// The level is not in the model's vocabulary (or not a level at all).
    Unsupported {
        requested: String,
        model: String,
        vocabulary: Vec<EffortLevel>,
    },
    /// The model offers no effort control: its record declares none, or the
    /// catalogue does not know the model at all.
    NoEffortControl { requested: String, model: String },
}

impl std::fmt::Display for EffortChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported {
                requested,
                model,
                vocabulary,
            } => write!(
                f,
                "invalid effort level \"{requested}\" for {model}; valid levels: {}",
                vocabulary
                    .iter()
                    .map(|level| level.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::NoEffortControl { requested, model } => write!(
                f,
                "effort level \"{requested}\" cannot be applied: {model} has no reasoning-effort \
                 control (declare `reasoning: true` on its models.json record if it does)"
            ),
        }
    }
}

impl std::error::Error for EffortChangeError {}

#[cfg(test)]
#[path = "effort_tests.rs"]
mod tests;
