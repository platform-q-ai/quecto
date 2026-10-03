//! Request and outcome of changing the session's active model (#1847).

use crate::application::catalogue::dto::{DefaultScope, PersistedDefault};
use crate::domain::catalogue::{ModelRef, PromptLimit, UnavailableReason};
use crate::domain::conversation::image_input::ImageInput;

/// The per-model limits the loop clamps to, each `None` unless the
/// catalogue declared it explicitly (a synthesized default never clamps),
/// and whether the model takes images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelLimits {
    pub max_output_tokens: Option<u32>,
    pub context_window: Option<usize>,
    /// How the provider bounds the prompt inside the window (#2405).
    pub prompt_limit: PromptLimit,
    /// What the model takes of a conversation's images (#2421): none unless
    /// its catalogue entry declares `image` input. A model with no entry
    /// takes none.
    pub image_input: ImageInput,
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
    /// Where the model stands in the generation just published (#2435).
    pub standing: CatalogueStanding,
}

/// Where a requested model stands in the published catalogue (#2435): the
/// entry a request for it reaches — through the router's routing rule —
/// lists it, refuses it for the account in use, or does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogueStanding {
    /// The provider the request reaches lists the model.
    Listed,
    /// The provider refused the model for the account in use, for this
    /// reason, earlier in this process.
    RefusedForAccount(String),
    /// The provider the request reaches lists other models but not this
    /// one: a retired built-in, or a typo.
    Unlisted,
    /// The request reaches no provider the catalogue lists models for: an
    /// endpoint whose models it cannot enumerate (an open-router prefix, an
    /// `openai_compatible` endpoint), or no provider at all.
    UncataloguedProvider,
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
    /// The requested id is not a `provider/model` reference, so there is
    /// no qualified id to record.
    Unqualified { model: String },
    /// The published catalogue lists no models for the reference's
    /// provider — unconfigured, or configured but with no entries yet (a
    /// failed source, a custom endpoint before its first `refresh_models`).
    /// Known-ness is decided from the catalogue entries, so the message
    /// says exactly that and names both remedies.
    UnknownProvider { model: String, provider: String },
    /// The model names a provider this harness cannot route to (#2126):
    /// switching would fail the next request, so the session keeps its
    /// model. `configured` lists the providers it can reach.
    Unroutable {
        model: String,
        provider: String,
        configured: Vec<String>,
    },
    /// The provider refused the model for the account in use earlier in
    /// this process (#2435): switching would fail every request, so the
    /// session keeps its model.
    RefusedForAccount { model: String, reason: String },
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
                "cannot persist `{model}` as a default: it is not a provider/model reference (a \
                 bare id would route to the first configured provider on a later start); select \
                 it as provider/model"
            ),
            Self::UnknownProvider { model, provider } => write!(
                f,
                "cannot persist `{model}` as a default: the published catalogue lists no models \
                 for `{provider}`; configure the provider or refresh the catalogue first, or \
                 choose a listed provider/model"
            ),
            Self::Unroutable {
                model,
                provider,
                configured,
            } => write!(
                f,
                "cannot switch to `{model}`: provider `{provider}` is not configured in this \
                 harness; configured providers: {}. Choose one of them as provider/model",
                configured.join(", ")
            ),
            Self::RefusedForAccount { model, reason } => write!(
                f,
                "cannot switch to `{model}`: the provider refused it for this account or auth \
                 mode earlier in this process ({reason}). Choose another model from list_models"
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
