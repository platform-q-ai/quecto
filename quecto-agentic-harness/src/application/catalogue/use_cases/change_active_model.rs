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
//!
//! A switch may also record the model as a configured default (#2024 S2)
//! — the repository overlay or the global file — through the
//! [`ModelDefaultPersistence`] port this use case owns. The record comes
//! before the apply: a refused record leaves the session as it was.

use std::sync::Arc;

use crate::application::catalogue::dto::{
    CatalogueStanding, ModelLimits, ModelSelectionVerdict, ModelSwitchError, ModelSwitchPlan,
    ModelSwitched,
};
use crate::application::catalogue::ports::{
    CatalogueInputsLoader, DefaultScope, ModelDefaultPersistence, ModelRuntime, PersistedDefault,
    RuntimeSnapshotSource,
};
use crate::application::catalogue::use_cases::ChangeReasoningEffort;
use crate::application::catalogue::{CatalogueSnapshotStore, ResolveCatalogueUseCase};
use crate::application::provider_runtime::{SelectionError, select_in_runtime};
use crate::domain::catalogue::{CatalogueEntry, CatalogueSnapshot, ModelRef, UnavailableReason};
use crate::domain::conversation::image_input::ImageInput;
use crate::domain::provider::{
    ModelRoute, parse_qualified_model, provider_prefix_matches, route_model,
};

pub struct ChangeActiveModel {
    inputs: Arc<dyn CatalogueInputsLoader>,
    store: CatalogueSnapshotStore,
    runtime: Arc<dyn RuntimeSnapshotSource>,
    effort: Arc<ChangeReasoningEffort>,
    persistence: Arc<dyn ModelDefaultPersistence>,
}

impl ChangeActiveModel {
    pub fn new(
        inputs: Arc<dyn CatalogueInputsLoader>,
        store: CatalogueSnapshotStore,
        runtime: Arc<dyn RuntimeSnapshotSource>,
        effort: Arc<ChangeReasoningEffort>,
        persistence: Arc<dyn ModelDefaultPersistence>,
    ) -> Self {
        Self {
            inputs,
            store,
            runtime,
            effort,
            persistence,
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
        let route_order = self
            .runtime
            .current_runtime()
            .map(|runtime| runtime.provider.route_order());
        let reached = Self::entry_for(&resolved.snapshot, model, route_order.as_deref());
        let limits = Self::limits_of(reached);
        let standing = Self::standing_of(
            reached,
            Self::provider_lists_models(&resolved.snapshot, model, route_order.as_deref()),
        );
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
            standing,
        }
    }

    /// The limits a run starts with for `model`: the same read a later
    /// switch performs.
    pub fn startup_limits(&self, model: &str) -> ModelLimits {
        self.plan(model).limits
    }

    /// Plan and apply: the loop runs on `model` with its limits from the
    /// next turn on, and its effort is reset for the new model. Refused, like
    /// every switch, when no configured provider can route it (#2126).
    pub fn execute(
        &self,
        runtime: &mut dyn ModelRuntime,
        model: &str,
    ) -> Result<ModelSwitched, ModelSwitchError> {
        self.execute_with_default(runtime, model, None)
    }

    /// Plan, record the model as the configured default of `persist` when
    /// asked, then apply. The recorded id is the qualified `provider/model`
    /// the plan resolved, and its provider must be one the generation just
    /// published lists models for: a bare id (which a later start would
    /// route to the first configured provider) and a provider the
    /// published catalogue lists no models for (unconfigured — every later
    /// start there would fail its first prompt — or not yet refreshed)
    /// are not recorded. The model itself need not be enumerated or
    /// runnable now — open-router prefixes accept ids the catalogue cannot
    /// list, and a credential may arrive later — so the verdict stays a
    /// verdict. Neither a refused id nor a record the adapter refuses
    /// changes the session.
    pub fn execute_with_default(
        &self,
        runtime: &mut dyn ModelRuntime,
        model: &str,
        persist: Option<DefaultScope>,
    ) -> Result<ModelSwitched, ModelSwitchError> {
        let plan = self.plan(model);
        // A model the provider refused for the account in use fails every
        // request: the session keeps its model (#2435).
        if let CatalogueStanding::RefusedForAccount(reason) = &plan.standing {
            return Err(ModelSwitchError::RefusedForAccount {
                model: plan.model.clone(),
                reason: reason.clone(),
            });
        }
        // Every switch, persisted or not, must land on a provider this
        // harness can reach, or the next request fails (#2126).
        if let crate::application::providers::ports::RouteCheck::UnknownProvider {
            provider,
            configured,
        } = runtime.route_check(&plan.model)
        {
            return Err(ModelSwitchError::Unroutable {
                model: plan.model.clone(),
                provider,
                configured,
            });
        }
        let persisted = match persist {
            None => None,
            Some(scope) => {
                let qualified = self.qualified(&plan)?;
                Some(
                    self.persistence
                        .persist_model(scope, &qualified)
                        .map_err(|reason| ModelSwitchError::Persist {
                            model: qualified,
                            scope,
                            reason,
                        })?,
                )
            }
        };
        Ok(self.apply(runtime, plan, persisted))
    }

    fn apply(
        &self,
        runtime: &mut dyn ModelRuntime,
        plan: ModelSwitchPlan,
        persisted: Option<PersistedDefault>,
    ) -> ModelSwitched {
        runtime.apply_model(plan.model.clone(), plan.limits);
        // The reset reads the model the runtime now reports, so it can only
        // ever follow the switch.
        let active = runtime.model().to_string();
        let effort_changed = self.effort.reset_for_model_switch(runtime, &active);
        ModelSwitched {
            plan,
            effort_changed,
            persisted,
        }
    }

    /// The `provider/model` id a plan stands for, if it names one on a
    /// provider the published catalogue lists models for.
    fn qualified(&self, plan: &ModelSwitchPlan) -> Result<String, ModelSwitchError> {
        let reference =
            ModelRef::parse_qualified(&plan.model).map_err(|_| ModelSwitchError::Unqualified {
                model: plan.model.clone(),
            })?;
        // The router matches provider prefixes case-insensitively; the
        // record carries the catalogue's own spelling.
        let snapshot = self.store.current();
        let provider = snapshot
            .entries()
            .iter()
            .map(|entry| entry.provider.id.as_str())
            .find(|known| known.eq_ignore_ascii_case(reference.provider().as_str()))
            .ok_or_else(|| ModelSwitchError::UnknownProvider {
                model: plan.model.clone(),
                provider: reference.provider().as_str().to_string(),
            })?;
        Ok(format!("{provider}/{}", reference.model()))
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

    /// The provider and model id a request for `model` reaches (#2421
    /// round 2 L1): the routing rule the router sends by picks the provider
    /// among the published runtime's `route_order` (a bare id goes to the
    /// first, whether or not it lists it). Before a runtime is published, a
    /// `provider/model` id reaches the provider its prefix names and a bare
    /// id none: no router yet says where it goes (round 3 L2, fail closed).
    fn route<'m>(model: &'m str, route_order: Option<&[String]>) -> Option<(String, &'m str)> {
        match route_order {
            Some(order) => {
                let names: Vec<&str> = order.iter().map(String::as_str).collect();
                match route_model(model, &names) {
                    ModelRoute::To { provider, model } => Some((provider.to_string(), model)),
                    ModelRoute::UnknownProvider { .. } | ModelRoute::NoProviders => None,
                }
            }
            None => parse_qualified_model(model).map(|(prefix, id)| (prefix.to_string(), id)),
        }
    }

    /// The entry of the model a request for `model` reaches: that
    /// provider's entry for the id. None when the provider does not list
    /// the model.
    fn entry_for<'s>(
        snapshot: &'s CatalogueSnapshot,
        model: &str,
        route_order: Option<&[String]>,
    ) -> Option<&'s CatalogueEntry> {
        let (provider, id) = Self::route(model, route_order)?;
        snapshot.entries().iter().find(|entry| {
            entry.model.reference.model().as_str() == id
                && provider_prefix_matches(&provider, entry.provider.id.as_str())
        })
    }

    /// Whether the provider a request for `model` reaches lists any model
    /// (#2435): one that lists none (an endpoint the catalogue cannot
    /// enumerate) says nothing of a model it does not list.
    fn provider_lists_models(
        snapshot: &CatalogueSnapshot,
        model: &str,
        route_order: Option<&[String]>,
    ) -> bool {
        Self::route(model, route_order).is_some_and(|(provider, _)| {
            snapshot
                .entries()
                .iter()
                .any(|entry| provider_prefix_matches(&provider, entry.provider.id.as_str()))
        })
    }

    /// Where the model a request reaches stands (#2435): refused for the
    /// account in use when its entry carries that reason, listed when it
    /// has an entry; without one, unlisted when its provider lists other
    /// models, else on a provider the catalogue does not enumerate.
    fn standing_of(
        entry: Option<&CatalogueEntry>,
        provider_lists_models: bool,
    ) -> CatalogueStanding {
        let Some(entry) = entry else {
            return match provider_lists_models {
                true => CatalogueStanding::Unlisted {
                    provider: String::new(),
                },
                false => CatalogueStanding::UncataloguedProvider,
            };
        };
        entry
            .model
            .availability
            .reasons()
            .iter()
            .find_map(|reason| match reason {
                UnavailableReason::RefusedForAccount(why) => {
                    Some(CatalogueStanding::RefusedForAccount(why.clone()))
                }
                _ => None,
            })
            .unwrap_or(CatalogueStanding::Listed)
    }

    /// Only explicitly declared values clamp: a synthesized default is not a
    /// real limit. A model the catalogue does not hold has none, and takes
    /// no image.
    fn limits_of(entry: Option<&CatalogueEntry>) -> ModelLimits {
        let Some(entry) = entry else {
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
            prompt_limit: capabilities.prompt_limit,
            image_input: ImageInput::declared(
                &capabilities.input_modalities,
                &entry.provider.transport,
            ),
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
