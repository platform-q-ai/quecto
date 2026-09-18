//! The roster and launch policy agree on what `container: true` selects
//! (#2024 S4c review): over one in-memory effective set, the entry
//! `ListContainerConfigs` marks default is exactly the entry
//! `SelectContainerConfig` launches for `container: true` with no name —
//! and where the launch is refused (the overlay withheld, more than one
//! labelled default, the default's argv broken, an empty set) the
//! inventory marks none. An agent that reads the roster and then launches
//! is never told one thing and served another.
use std::sync::Arc;

use quecto::application::environments::use_cases::ListContainerConfigs;
use quecto::application::subagents::dto::{
    ContainerConfigSource, ContainerConfigsError, ContainerLaunchConfig,
    EffectiveContainerConfigSet, SelectContainerConfigRequest,
};
use quecto::application::subagents::ports::EffectiveContainerConfigs;
use quecto::application::subagents::use_cases::SelectContainerConfig;
use quecto::composition::container_configs::build_container_script_integrity;
use quecto::infrastructure::config::container_config_roster::EffectiveConfigRoster;

struct InMemory(EffectiveContainerConfigSet);

impl EffectiveContainerConfigs for InMemory {
    fn effective_container_configs(
        &self,
        _source: &ContainerConfigSource,
    ) -> Result<EffectiveContainerConfigSet, ContainerConfigsError> {
        Ok(self.0.clone())
    }
}

fn config(name: &str, default: bool, create: &[&str]) -> ContainerLaunchConfig {
    ContainerLaunchConfig {
        name: name.to_string(),
        default,
        create: create.iter().map(|s| s.to_string()).collect(),
        cleanup: vec!["/bin/cleanup".into()],
        exec: vec![],
        kill: vec![],
        inspect: vec![],
        repo_bound: false,
        repository: None,
    }
}

fn set(configs: Vec<ContainerLaunchConfig>, overlay_withheld: bool) -> EffectiveContainerConfigSet {
    EffectiveContainerConfigSet {
        configs,
        diagnostics: vec![],
        overlay_withheld,
    }
}

/// The default the inventory reports and the config the launch selects,
/// over the same set, as names.
fn both(set: EffectiveContainerConfigSet) -> (Option<String>, Option<String>) {
    let configs: Arc<dyn EffectiveContainerConfigs> = Arc::new(InMemory(set));
    let roster = Arc::new(EffectiveConfigRoster::new(
        Arc::clone(&configs),
        Arc::new(|| String::from("rev")),
    ));
    let listed = ListContainerConfigs::new(roster)
        .execute()
        .unwrap()
        .default_entry()
        .map(|entry| entry.name.clone());
    let selected = SelectContainerConfig::new(configs, build_container_script_integrity())
        .execute(&SelectContainerConfigRequest {
            source: ContainerConfigSource::LaunchingAgent,
            name: None,
        })
        .ok()
        .map(|selected| selected.config.name);
    (listed, selected)
}

fn assert_agree(case: &str, set: EffectiveContainerConfigSet, expected: Option<&str>) {
    let (listed, selected) = both(set);
    assert_eq!(
        listed, selected,
        "{case}: roster {listed:?} vs launch {selected:?}"
    );
    assert_eq!(listed.as_deref(), expected, "{case}");
}

#[test]
fn one_labelled_default_is_listed_and_launched() {
    assert_agree(
        "one default",
        set(
            vec![
                config("b", false, &["/bin/create"]),
                config("a", true, &["/bin/create"]),
            ],
            false,
        ),
        Some("a"),
    );
}

#[test]
fn a_withheld_overlay_clears_the_default_and_refuses_the_launch() {
    assert_agree(
        "withheld",
        set(vec![config("a", true, &["/bin/create"])], true),
        None,
    );
}

#[test]
fn more_than_one_labelled_default_clears_them_and_refuses_the_launch() {
    assert_agree(
        "multiple defaults",
        set(
            vec![
                config("a", true, &["/bin/create"]),
                config("b", true, &["/bin/create"]),
            ],
            false,
        ),
        None,
    );
    // A broken entry still counts as a labelled default: launch policy
    // refuses the implicit selection rather than falling through to the
    // launchable one.
    assert_agree(
        "multiple defaults, one broken",
        set(
            vec![config("a", true, &[]), config("b", true, &["/bin/create"])],
            false,
        ),
        None,
    );
}

#[test]
fn a_default_with_a_broken_argv_is_not_listed_as_default_and_is_refused() {
    assert_agree(
        "missing create",
        set(
            vec![config("a", true, &[]), config("b", false, &["/bin/create"])],
            false,
        ),
        None,
    );
    assert_agree(
        "unsafe argv",
        set(vec![config("a", true, &["/bin/create", ""])], false),
        None,
    );
}

#[test]
fn an_empty_set_has_no_default_and_no_launch() {
    assert_agree("empty", set(vec![], false), None);
}
