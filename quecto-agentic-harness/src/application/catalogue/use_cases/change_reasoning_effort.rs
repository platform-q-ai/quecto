//! Change reasoning effort (#1848, #1996): select a provider-valid effort
//! level for the subsequent turns of a session, and answer every surface
//! that needs to know which levels a model accepts.
//!
//! One rule for every trigger — UDS/child `set_effort`, startup `--effort`
//! and `agents.defaults.effort`, spawn `effort`, the `get_state` listing
//! the selector shows, and the reset on a model switch: the vocabulary is
//! the catalogue's per-model capability, read through a port. A level the
//! model does not accept is refused here, so nothing outside the model's
//! vocabulary ever reaches a provider adapter.

use std::sync::Arc;

use crate::application::catalogue::dto::{
    EffortChangeError, EffortChangeOutcome, EffortChangeRequest,
};
use crate::application::catalogue::ports::{EffortRuntime, EffortVocabularySource};
use crate::domain::provider::EffortLevel;

pub struct ChangeReasoningEffort {
    vocabulary: Arc<dyn EffortVocabularySource>,
}

impl ChangeReasoningEffort {
    pub fn new(vocabulary: Arc<dyn EffortVocabularySource>) -> Self {
        Self { vocabulary }
    }

    /// The levels `model` accepts, in ascending order; empty when the model
    /// offers no effort control or is unknown to the catalogue.
    pub fn choices(&self, model: &str) -> Vec<EffortLevel> {
        self.vocabulary.effort_vocabulary(model).unwrap_or_default()
    }

    /// Validate `level` (as typed) against `model`'s vocabulary.
    pub fn validate(&self, model: &str, level: &str) -> Result<EffortLevel, EffortChangeError> {
        let vocabulary = self.choices(model);
        if vocabulary.is_empty() {
            return Err(EffortChangeError::NoEffortControl {
                requested: level.to_string(),
                model: model.to_string(),
            });
        }
        EffortLevel::parse(level)
            .filter(|parsed| vocabulary.contains(parsed))
            .ok_or_else(|| EffortChangeError::Unsupported {
                requested: level.to_string(),
                model: model.to_string(),
                vocabulary,
            })
    }

    /// The level to carry into `model`: `level` itself when the model
    /// accepts it, otherwise nothing (the provider's default). Used where a
    /// level chosen elsewhere meets a model — the configured default at
    /// startup, and the reset on a model switch.
    pub fn admit(&self, model: &str, level: Option<EffortLevel>) -> Option<EffortLevel> {
        level.filter(|level| self.choices(model).contains(level))
    }

    /// Apply a requested level to the session. Refused levels leave the
    /// runtime untouched.
    pub fn execute(
        &self,
        runtime: &mut dyn EffortRuntime,
        request: &EffortChangeRequest,
    ) -> Result<EffortChangeOutcome, EffortChangeError> {
        let effective = self.validate(&request.model, &request.level)?;
        if runtime.effort() != Some(effective) {
            runtime.apply_effort(Some(effective));
        }
        Ok(EffortChangeOutcome {
            effective,
            vocabulary: self.choices(&request.model),
        })
    }
}

impl std::fmt::Debug for ChangeReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeReasoningEffort")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "change_reasoning_effort_tests.rs"]
mod tests;
