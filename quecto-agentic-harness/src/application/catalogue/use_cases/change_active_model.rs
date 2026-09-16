//! Change the active model (#1847): use another provider/model for the
//! subsequent turns of a session — the UDS/child `set_model` command and,
//! at startup, the `--model` flag or configured default.
//!
//! One published generation answers everything a switch needs: the inputs
//! are loaded afresh and republished (so a model added to `models.json`
//! since startup is switchable without a refresh), the model's declared
//! limits come from that generation, the runtime's selection verdict from
//! the same generation, and the session effort is reset through the
//! change-reasoning-effort use case for the model now active.

use std::sync::Arc;

use crate::application::catalogue::dto::{
    ModelLimits, ModelSelectionVerdict, ModelSwitchPlan, ModelSwitched,
};
use crate::application::catalogue::ports::{
    CatalogueInputsLoader, ModelRuntime, RuntimeSnapshotSource,
};
use crate::application::catalogue::use_cases::ChangeReasoningEffort;
use crate::application::catalogue::{CatalogueSnapshotStore, ResolveCatalogueUseCase};
use crate::application::provider_runtime::{SelectionError, select_in_runtime};
use crate::domain::catalogue::{CatalogueSnapshot, ModelRef};

pub struct ChangeActiveModel {
    inputs: Arc<dyn CatalogueInputsLoader>,
    store: CatalogueSnapshotStore,
    runtime: Arc<dyn RuntimeSnapshotSource>,
    effort: Arc<ChangeReasoningEffort>,
}

impl ChangeActiveModel {
    pub fn new(
        inputs: Arc<dyn CatalogueInputsLoader>,
        store: CatalogueSnapshotStore,
        runtime: Arc<dyn RuntimeSnapshotSource>,
        effort: Arc<ChangeReasoningEffort>,
    ) -> Self {
        Self {
            inputs,
            store,
            runtime,
            effort,
        }
    }

    /// Republish the catalogue from the current inputs and plan a switch to
    /// `model`: its declared limits and the runtime's verdict, both from the
    /// generation just published.
    pub fn plan(&self, model: &str) -> ModelSwitchPlan {
        let loaded = self.inputs.load();
        let resolved = ResolveCatalogueUseCase.resolve_and_publish(
            &loaded.sources(),
            loaded.credentials(),
            &self.store,
        );
        let reference = ModelRef::parse_qualified(model).ok();
        let limits = reference
            .as_ref()
            .map(|reference| Self::limits_in(&resolved.snapshot, reference))
            .unwrap_or_default();
        let verdict = match reference {
            Some(reference) => self.verdict(&reference),
            None => ModelSelectionVerdict::Unknown {
                reference: model.to_string(),
            },
        };
        ModelSwitchPlan {
            model: model.to_string(),
            limits,
            verdict,
        }
    }

    /// The limits a run starts with for `model`: the same read a later
    /// switch performs.
    pub fn startup_limits(&self, model: &str) -> ModelLimits {
        self.plan(model).limits
    }

    /// Plan and apply: the loop runs on `model` with its limits from the
    /// next turn on, and its effort is reset for the new model.
    pub fn execute(&self, runtime: &mut dyn ModelRuntime, model: &str) -> ModelSwitched {
        let plan = self.plan(model);
        runtime.apply_model(plan.model.clone(), plan.limits);
        let effort_changed = self.effort.reset_for_model_switch(runtime, &plan.model);
        ModelSwitched {
            plan,
            effort_changed,
        }
    }

    /// The published runtime's verdict on `reference` (#1573): known and
    /// runnable on its provider, unknown, not runnable for structured
    /// reasons, or no runtime composed yet.
    fn verdict(&self, reference: &ModelRef) -> ModelSelectionVerdict {
        let runtime = self.runtime.current_runtime();
        match select_in_runtime(runtime.as_deref(), reference) {
            Ok(selection) => ModelSelectionVerdict::Runnable {
                provider: selection.entry.provider.id.as_str().to_string(),
                generation: selection.generation,
            },
            Err(SelectionError::UnknownModel { reference }) => {
                ModelSelectionVerdict::Unknown { reference }
            }
            Err(SelectionError::NotRunnable { reference, reasons }) => {
                ModelSelectionVerdict::NotRunnable { reference, reasons }
            }
            Err(SelectionError::NoRuntime) => ModelSelectionVerdict::NoRuntime,
        }
    }

    /// Only explicitly declared values clamp: a synthesized default is not a
    /// real limit.
    fn limits_in(snapshot: &CatalogueSnapshot, reference: &ModelRef) -> ModelLimits {
        let Some(entry) = snapshot.find(reference) else {
            return ModelLimits::default();
        };
        let capabilities = &entry.model.capabilities;
        ModelLimits {
            max_output_tokens: capabilities
                .max_output_tokens_explicit
                .then_some(capabilities.max_output_tokens),
            context_window: capabilities
                .context_window_explicit
                .then_some(capabilities.context_window as usize),
        }
    }
}

impl std::fmt::Debug for ChangeActiveModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeActiveModel").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "change_active_model_tests.rs"]
mod tests;
