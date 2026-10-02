//! The configuration an agent's build loads (#2313 review L5), in one
//! place: the build itself, and the event-log switch decided before
//! admission (`event_log::decided_with`), read it over the same layers,
//! overlay trust and `QUECTO_*` overrides, so the two never disagree.
use std::collections::HashMap;
use std::path::Path;

use super::AgentFlags;
use crate::application::configuration::dto::ConfigSelection;
use crate::interface::cli::config_loading::{LoadedConfig, load_selected_config};

/// The configuration `selection` names, over `base_dir`, realized with
/// `env_overrides`: an explicit file that must exist but does not, a
/// build without composition's configuration capability, or a layer that
/// does not load is an error (its text, without a newline). An overlay not
/// trusted yet is offered only when `prompt_for_trust`.
pub(super) fn load(
    base_dir: &Path,
    selection: &ConfigSelection,
    flags: &AgentFlags,
    prompt_for_trust: bool,
    env_overrides: &HashMap<String, String>,
) -> Result<LoadedConfig, String> {
    // An explicit --config must exist; only a missing GLOBAL config falls
    // back to zero-config defaults.
    if let Some(missing) =
        crate::interface::cli::selected_config_missing(selection.path(), selection.must_exist())
    {
        return Err(missing);
    }
    let Some(build_configuration) = flags.configuration else {
        return Err("agent: configuration capability not composed".to_owned());
    };
    let mut loaded = load_selected_config(
        build_configuration,
        base_dir,
        selection,
        prompt_for_trust,
        env_overrides,
        flags.admission_context.is_some(),
    )?;
    // What the launching agent handed down fills what this agent's own
    // configuration leaves unset (#2403).
    if let Some(inherited) = flags.inherited_context_mode.as_deref() {
        let defaults = &mut loaded.config.agents.defaults;
        defaults
            .inherit_context_mode(inherited)
            .map_err(|error| format!("agent: {error}"))?;
    }
    Ok(loaded)
}
