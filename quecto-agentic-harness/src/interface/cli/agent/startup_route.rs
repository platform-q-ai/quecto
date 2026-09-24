//! #2126: the startup model must reach a configured provider. A spawned
//! child with an unroutable model fails at once with the configured
//! providers named, instead of failing its first prompt; the root harness
//! still starts, with a warning, so a bad default never locks the owner out.
use crate::application::providers::ports::RouteCheck;

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

#[cfg(test)]
#[path = "startup_route_tests.rs"]
mod tests;
