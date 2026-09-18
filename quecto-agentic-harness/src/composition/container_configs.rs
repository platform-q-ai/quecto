//! Container config selection composition (#2024 S4a): the launch policy's
//! `SelectContainerConfig` over the configuration capability's effective
//! configuration for the launching agent's own selection — its base file
//! with its *checkout's* trusted overlay merged in, never the quecto base
//! directory — and, for a spawn call that names a file, that file alone.
//! One overlay, one trust record: the loaders resolve through the same
//! handles `quecto config get --effective` and the agent build use, with
//! a never-prompting trust decision (a spawn runs on the dispatch loop,
//! not a terminal), so an untrusted overlay travels as a diagnostic and
//! `quecto config trust` is the one approval path.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::application::configuration::dto::{ConfigSelection, EffectiveConfigError, OverlayState};
use crate::application::environments::use_cases::ListContainerConfigs;
use crate::application::subagents::ports::EffectiveContainerConfigs;
use crate::application::subagents::use_cases::SelectContainerConfig;
use crate::infrastructure::config::container_config_roster::EffectiveConfigRoster;
use crate::infrastructure::config::container_configs::{
    ContainerConfigsFromEffectiveConfig, ExplicitConfigLoader, LaunchingAgentConfigLoader,
    ResolvedConfig,
};
use crate::interface::cli::configuration_handles::{
    ConfigurationEnvironment, ConfigurationHandles,
};
use crate::interface::cli::container_config_handles::ContainerConfigHandles;

/// The selection use case a launcher holds, over `launching_agent` (the
/// launching agent's own configuration selection; `None` for a launcher
/// composed without one) with trust recorded under `base_dir`.
pub fn build_container_config_selection(
    base_dir: &Path,
    launching_agent: Option<ConfigSelection>,
) -> Arc<SelectContainerConfig> {
    build_container_config_handles(base_dir, launching_agent).selection
}

/// The selection AND the discovery query (#2024 S4c) over one effective
/// container-config adapter, so `container: true`, `get_container_configs`
/// and the spawn description's roster line read the same layers.
pub fn build_container_config_handles(
    base_dir: &Path,
    launching_agent: Option<ConfigSelection>,
) -> ContainerConfigHandles {
    let configs = build_effective_container_configs(base_dir, launching_agent.clone());
    ContainerConfigHandles {
        selection: Arc::new(SelectContainerConfig::new(configs.clone())),
        roster: Arc::new(ListContainerConfigs::new(Arc::new(
            EffectiveConfigRoster::new(configs, roster_revision_probe(base_dir, launching_agent)),
        ))),
    }
}

/// The files whose change could change the roster: the launching agent's
/// configuration layers (base, overlay, retired local file) and the trust
/// record under `base_dir`. Probed by metadata only, on every render.
fn roster_revision_probe(
    base_dir: &Path,
    launching_agent: Option<ConfigSelection>,
) -> crate::infrastructure::config::container_config_roster::RosterRevisionProbe {
    let mut paths =
        vec![base_dir.join(crate::infrastructure::config::persistence::TRUST_RECORD_FILE_NAME)];
    match launching_agent {
        Some(ConfigSelection::Explicit(path)) => paths.push(path),
        Some(ConfigSelection::Layered(layers)) => {
            paths.push(layers.global);
            paths.extend(layers.overlay);
            paths.extend(layers.legacy_local);
        }
        None => {}
    }
    Arc::new(move || crate::infrastructure::config::container_config_roster::file_revision(&paths))
}

/// The handles an agent run's spawn and agent_cmd tools hold (#2024 S4a,
/// S4c), over the run's own configuration selection — the layers the
/// agent build resolved for its working directory — with trust under
/// `base_dir`. `main` hands this builder to the CLI entry point; the agent
/// build invokes it once beside the other capability builders.
pub fn build_agent_container_config_handles(
    base_dir: &Path,
    selection: &ConfigSelection,
) -> ContainerConfigHandles {
    build_container_config_handles(base_dir, Some(selection.clone()))
}

/// The discovery port adapter alone, for the contract suite.
pub fn build_container_config_roster(
    base_dir: &Path,
    launching_agent: Option<ConfigSelection>,
) -> Arc<dyn crate::application::environments::ports::ContainerConfigRoster> {
    Arc::new(EffectiveConfigRoster::new(
        build_effective_container_configs(base_dir, launching_agent.clone()),
        roster_revision_probe(base_dir, launching_agent),
    ))
}

/// The port adapter alone, for the contract suite.
pub fn build_effective_container_configs(
    base_dir: &Path,
    launching_agent: Option<ConfigSelection>,
) -> Arc<dyn EffectiveContainerConfigs> {
    let handles = super::configuration::build_configuration_handles(&ConfigurationEnvironment {
        base_dir: base_dir.to_path_buf(),
        prompt_for_trust: false,
    });
    let launching_agent: Option<LaunchingAgentConfigLoader> = launching_agent.map(|selection| {
        let handles = handles.clone();
        let loader: LaunchingAgentConfigLoader = Arc::new(move || resolve(&handles, &selection));
        loader
    });
    let explicit: ExplicitConfigLoader =
        Arc::new(move |path| resolve(&handles, &ConfigSelection::Explicit(path.to_path_buf())));
    Arc::new(ContainerConfigsFromEffectiveConfig::new(
        launching_agent,
        explicit,
    ))
}

/// The selection resolved and realized without environment overrides:
/// `container_configs` has none, and a spawn must not depend on this
/// process's `QUECTO_*` environment for the argv it executes. A file that
/// cannot be read or parsed keeps the `invalid container_configs
/// configuration:` prefix a spawn caller has always seen; a validation
/// failure already names the file and the section, so it travels bare.
fn resolve(
    handles: &ConfigurationHandles,
    selection: &ConfigSelection,
) -> Result<ResolvedConfig, String> {
    let effective = handles
        .resolve
        .execute(selection)
        .map_err(|error| match error {
            EffectiveConfigError::Missing(_)
            | EffectiveConfigError::Read { .. }
            | EffectiveConfigError::Parse { .. }
            | EffectiveConfigError::NotAnObject(_) => {
                format!("invalid container_configs configuration: {error}")
            }
            EffectiveConfigError::GlobalOnlyKey { .. }
            | EffectiveConfigError::Invalid { .. }
            | EffectiveConfigError::InvalidMerge { .. } => error.to_string(),
        })?;
    let diagnostics = effective.sources.diagnostics();
    let overlay_withheld = effective
        .sources
        .overlay
        .as_ref()
        .is_some_and(|report| withholds_container_configs(&report.state));
    let overlay_entries = effective
        .overlay_document
        .as_ref()
        .and_then(|overlay| overlay.get("container_configs"))
        .and_then(|section| section.as_object())
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default();
    let config = (handles.realize)(effective.document, &HashMap::new())?;
    Ok(ResolvedConfig {
        config,
        diagnostics,
        overlay_withheld,
        overlay_entries,
    })
}

/// Whether an overlay that was not applied leaves the default container
/// config unknown: a refused document (not a regular file) or one the
/// checks turned away declares nothing knowable; an untrusted document
/// that parsed withholds the default only when it carries a
/// `container_configs` section — an overlay that only pins, say,
/// `agents.defaults` cannot have changed it, so its diagnostic travels as
/// a warning and the global default launches. Judged on the resolver's
/// already-parsed top-level keys; nothing of the overlay is applied.
fn withholds_container_configs(state: &OverlayState) -> bool {
    match state {
        OverlayState::Refused { .. }
        | OverlayState::Untrusted {
            problem: Some(_), ..
        } => true,
        OverlayState::Untrusted {
            problem: None,
            sections,
            ..
        } => sections
            .iter()
            .any(|section| section == "container_configs"),
        OverlayState::Applied | OverlayState::Absent => false,
    }
}

#[cfg(test)]
#[path = "container_configs_tests.rs"]
mod tests;
