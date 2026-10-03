//! #2126: the startup model must reach a configured provider. A spawned
//! child with an unroutable model fails at once with the configured
//! providers named, instead of failing its first prompt; the root harness
//! still starts, with a warning, so a bad default never locks the owner out.
//! #2435: a startup model the catalogue no longer lists (a retired
//! built-in) or one refused for the account starts with a warning too.
use crate::application::catalogue::dto::{CatalogueStanding, ModelLimits};
use crate::application::providers::ports::RouteCheck;
use crate::interface::cli::catalogue_handles::CatalogueHandles;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StartupRoute {
    Proceed,
    Warn(String),
    Refuse(String),
}

pub(super) fn startup_route(check: RouteCheck, model: &str, spawned: bool) -> StartupRoute {
    match check {
        RouteCheck::Routable => StartupRoute::Proceed,
        RouteCheck::UnknownProvider {
            provider,
            configured,
        } => {
            let message = format!(
                "model `{model}`: provider `{provider}` is not configured in this harness; \
                 configured providers: {}",
                configured.join(", ")
            );
            match spawned {
                true => StartupRoute::Refuse(format!(
                    "agent: cannot start with {message}. Spawn it with `model` set to one of \
                     them as provider/model"
                )),
                false => StartupRoute::Warn(format!(
                    "agent: warning: {message}. Prompts will fail until the model is changed"
                )),
            }
        }
    }
}

/// Applies the verdict: `None` stops the startup (a spawned child); a
/// warning joins the startup report. The owner also sees the router's
/// actionable refusal on the first prompt, which names the configured
/// providers.
pub(super) fn admit(
    check: RouteCheck,
    model: &str,
    spawned: bool,
    catalogue: &mut CatalogueHandles,
    stderr: &mut String,
) -> Option<()> {
    match startup_route(check, model, spawned) {
        StartupRoute::Proceed => Some(()),
        StartupRoute::Warn(message) => {
            warn(catalogue, stderr, message);
            Some(())
        }
        StartupRoute::Refuse(message) => {
            stderr.push_str(&format!("{message}\n"));
            None
        }
    }
}

/// #2435: the warning a startup model earns from where it stands in the
/// catalogue, if any.
pub(super) fn standing_warning(standing: &CatalogueStanding, model: &str) -> Option<String> {
    match standing {
        CatalogueStanding::Listed | CatalogueStanding::UncataloguedProvider => None,
        CatalogueStanding::Unlisted { provider } => Some(format!(
            "agent: warning: provider `{provider}` does not list model `{model}`; it is sent \
             as-is with no known limits. Declare it in models.json, or choose a listed model \
             (list_models, /model)"
        )),
        CatalogueStanding::Retired { provider } => Some(format!(
            "agent: warning: model `{model}` was retired from the built-in models of \
             `{provider}` (#2435); it is sent as-is with no known limits. Declare it in \
             models.json to keep it, or choose a listed model (list_models, /model)"
        )),
        CatalogueStanding::RefusedForAccount(reason) => Some(format!(
            "agent: warning: model `{model}` was refused for this account or auth mode \
             ({reason}); choose a listed model (list_models, /model)"
        )),
    }
}

/// The startup model's limits (#935/#1044): its declared output cap
/// (clamping `max_tokens`) and context window (bounding the budget), read
/// from the change-active-model use case as a later `set_model` reads them
/// (#1847). Where the model stands in the catalogue joins the startup
/// report when it deserves a warning (#2435).
pub(super) fn startup_limits(
    catalogue: &mut CatalogueHandles,
    model: &str,
    stderr: &mut String,
) -> ModelLimits {
    let plan = catalogue.model.plan(model);
    if let Some(warning) = standing_warning(&plan.standing, model) {
        warn(catalogue, stderr, warning);
    }
    plan.limits
}

/// A startup warning goes to stderr and is kept on the run's catalogue
/// handles, which `get_state` reads it from for clients that never see
/// stderr (#2435 review round 1 L6).
fn warn(catalogue: &mut CatalogueHandles, stderr: &mut String, warning: String) {
    debug_assert!(warning.starts_with("agent: warning: "), "{warning}");
    stderr.push_str(&format!("{warning}\n"));
    catalogue.startup_warnings.push(warning);
}

#[cfg(test)]
#[path = "startup_route_tests.rs"]
mod tests;
