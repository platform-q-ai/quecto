//! Startup admission of the reasoning effort (#1848, #1996): the level a
//! run starts with is admitted against the startup model's catalogue
//! vocabulary through the change-reasoning-effort use case. An explicit
//! `--effort` the model does not accept refuses to start, naming the levels
//! it does; a configured `agents.defaults.effort` the model does not accept
//! is dropped with a warning (the provider default applies), so a global
//! default never reaches a model whose wire would silently ignore or reject
//! it.

use crate::application::catalogue::use_cases::ChangeReasoningEffort;
use crate::domain::provider::EffortLevel;
use crate::infrastructure::config::Config;

/// `Some(level)` to apply, `Some(None)` for the provider default, `None`
/// when an explicit flag was refused (the message is on `stderr`).
pub(super) fn admit(
    effort_control: &ChangeReasoningEffort,
    flag: Option<EffortLevel>,
    config: &Config,
    model: &str,
    stderr: &mut String,
) -> Option<Option<EffortLevel>> {
    if let Some(level) = flag {
        return match effort_control.validate(model, level.as_str()) {
            Ok(level) => Some(Some(level)),
            Err(error) => {
                stderr.push_str(&format!("agent: --effort: {error}\n"));
                None
            }
        };
    }
    let configured = config
        .agents
        .defaults
        .effort
        .as_deref()
        .and_then(EffortLevel::parse);
    let admitted = effort_control.admit(model, configured);
    if let (Some(configured), None) = (configured, admitted) {
        stderr.push_str(&format!(
            "WARNING: configured effort '{}' is not accepted by {model}; using the provider default\n",
            configured.as_str()
        ));
    }
    Some(admitted)
}

#[cfg(test)]
#[path = "startup_effort_tests.rs"]
mod tests;
