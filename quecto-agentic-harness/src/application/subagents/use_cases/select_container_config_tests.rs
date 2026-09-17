use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::SelectContainerConfig;
use crate::application::subagents::dto::{
    ContainerConfigSource, ContainerConfigsError, ContainerLaunchConfig,
    EffectiveContainerConfigSet, SelectContainerConfigError, SelectContainerConfigRequest,
};
use crate::application::subagents::ports::EffectiveContainerConfigs;

struct FakeConfigs {
    result: Result<EffectiveContainerConfigSet, ContainerConfigsError>,
    asked: Mutex<Vec<ContainerConfigSource>>,
}

impl EffectiveContainerConfigs for FakeConfigs {
    fn effective_container_configs(
        &self,
        source: &ContainerConfigSource,
    ) -> Result<EffectiveContainerConfigSet, ContainerConfigsError> {
        self.asked.lock().unwrap().push(source.clone());
        self.result.clone()
    }
}

fn entry(name: &str, default: bool) -> ContainerLaunchConfig {
    ContainerLaunchConfig {
        name: name.to_string(),
        default,
        create: vec!["/bin/create".into()],
        cleanup: vec!["/bin/cleanup".into()],
        exec: vec![],
        kill: vec![],
        inspect: vec![],
    }
}

fn use_case(
    result: Result<EffectiveContainerConfigSet, ContainerConfigsError>,
) -> (SelectContainerConfig, Arc<FakeConfigs>) {
    let fake = Arc::new(FakeConfigs {
        result,
        asked: Mutex::new(Vec::new()),
    });
    (SelectContainerConfig::new(fake.clone()), fake)
}

fn set(configs: Vec<ContainerLaunchConfig>, diagnostics: Vec<&str>) -> EffectiveContainerConfigSet {
    EffectiveContainerConfigSet {
        configs,
        diagnostics: diagnostics.into_iter().map(String::from).collect(),
    }
}

fn request(source: ContainerConfigSource, name: Option<&str>) -> SelectContainerConfigRequest {
    SelectContainerConfigRequest {
        source,
        name: name.map(String::from),
    }
}

#[test]
fn container_true_selects_the_one_default_and_carries_the_layer_diagnostics() {
    let (select, fake) = use_case(Ok(set(
        vec![entry("global", false), entry("repo", true)],
        vec!["overlay note"],
    )));
    let selected = select
        .execute(&request(ContainerConfigSource::LaunchingAgent, None))
        .unwrap();
    assert_eq!(selected.config.name, "repo");
    assert_eq!(selected.diagnostics, vec!["overlay note".to_string()]);
    assert_eq!(
        fake.asked.lock().unwrap().as_slice(),
        &[ContainerConfigSource::LaunchingAgent]
    );
}

#[test]
fn an_explicit_name_wins_regardless_of_labels() {
    let (select, _) = use_case(Ok(set(
        vec![entry("default", true), entry("other", false)],
        vec![],
    )));
    let selected = select
        .execute(&request(
            ContainerConfigSource::LaunchingAgent,
            Some("other"),
        ))
        .unwrap();
    assert_eq!(selected.config.name, "other");
}

#[test]
fn unknown_names_and_missing_defaults_enumerate_the_sorted_menu() {
    let (select, _) = use_case(Ok(set(
        vec![entry("zeta", false), entry("alpha", false)],
        vec![],
    )));
    let unknown = select
        .execute(&request(
            ContainerConfigSource::LaunchingAgent,
            Some("missing"),
        ))
        .unwrap_err();
    assert_eq!(
        unknown.to_string(),
        "unknown container config 'missing' (available container configs: alpha, zeta)"
    );
    let no_default = select
        .execute(&request(ContainerConfigSource::LaunchingAgent, None))
        .unwrap_err();
    assert_eq!(
        no_default.to_string(),
        "no container config is labeled \"default\": true (available container configs: alpha, zeta)"
    );
}

#[test]
fn an_empty_set_reads_as_none_configured() {
    let (select, _) = use_case(Ok(set(vec![], vec![])));
    let err = select
        .execute(&request(ContainerConfigSource::LaunchingAgent, None))
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "no container config is labeled \"default\": true (available container configs: none configured)"
    );
}

#[test]
fn a_relative_explicit_path_is_refused_before_anything_is_read() {
    let (select, fake) = use_case(Ok(set(vec![entry("default", true)], vec![])));
    let err = select
        .execute(&request(
            ContainerConfigSource::Explicit(PathBuf::from("relative.json")),
            None,
        ))
        .unwrap_err();
    assert_eq!(
        err,
        SelectContainerConfigError::RelativeConfigPath(PathBuf::from("relative.json"))
    );
    assert_eq!(
        err.to_string(),
        "container spawn requires an absolute trusted config path"
    );
    assert!(fake.asked.lock().unwrap().is_empty());
}

#[test]
fn an_absolute_explicit_path_is_asked_for_as_such() {
    let (select, fake) = use_case(Ok(set(vec![entry("default", true)], vec![])));
    select
        .execute(&request(
            ContainerConfigSource::Explicit(PathBuf::from("/abs/config.json")),
            None,
        ))
        .unwrap();
    assert_eq!(
        fake.asked.lock().unwrap().as_slice(),
        &[ContainerConfigSource::Explicit(PathBuf::from(
            "/abs/config.json"
        ))]
    );
}

#[test]
fn port_errors_are_relayed_in_their_own_words() {
    let (select, _) = use_case(Err(ContainerConfigsError::NoSource));
    let err = select
        .execute(&request(ContainerConfigSource::LaunchingAgent, None))
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "container spawn requires --config so container_configs can be loaded"
    );
    let (select, _) = use_case(Err(ContainerConfigsError::Invalid(
        "failed to load config /x: boom".into(),
    )));
    let err = select
        .execute(&request(ContainerConfigSource::LaunchingAgent, None))
        .unwrap_err();
    assert_eq!(err.to_string(), "failed to load config /x: boom");
}

#[test]
fn a_selected_entry_must_carry_runnable_argv() {
    let cases: Vec<(ContainerLaunchConfig, &str)> = vec![
        (
            ContainerLaunchConfig {
                create: vec![],
                ..entry("default", true)
            },
            "invalid container_configs configuration: missing create argv",
        ),
        (
            ContainerLaunchConfig {
                cleanup: vec![],
                ..entry("default", true)
            },
            "invalid container_configs configuration: missing cleanup argv",
        ),
        (
            ContainerLaunchConfig {
                create: vec!["bad\0arg".into()],
                ..entry("default", true)
            },
            "invalid container_configs configuration: unsafe argv",
        ),
        (
            ContainerLaunchConfig {
                inspect: vec![String::new()],
                ..entry("default", true)
            },
            "invalid container_configs configuration: unsafe argv",
        ),
    ];
    for (config, expected) in cases {
        let (select, _) = use_case(Ok(set(vec![config], vec![])));
        let err = select
            .execute(&request(ContainerConfigSource::LaunchingAgent, None))
            .unwrap_err();
        assert_eq!(err.to_string(), expected);
    }
    // A valid entry with every optional argv set passes.
    let (select, _) = use_case(Ok(set(
        vec![ContainerLaunchConfig {
            exec: vec!["/bin/exec".into()],
            kill: vec!["/bin/kill".into()],
            inspect: vec!["/bin/inspect".into()],
            ..entry("default", true)
        }],
        vec![],
    )));
    assert!(
        select
            .execute(&request(ContainerConfigSource::LaunchingAgent, None))
            .is_ok()
    );
}
