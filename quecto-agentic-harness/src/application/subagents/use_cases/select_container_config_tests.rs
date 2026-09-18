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
        repo_bound: false,
        repository: None,
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
        overlay_withheld: false,
    }
}

/// The set a checkout with an untrusted (or refused) overlay resolves to:
/// the global entries alone, the overlay's diagnostic, and the flag.
fn withheld(configs: Vec<ContainerLaunchConfig>, diagnostic: &str) -> EffectiveContainerConfigSet {
    EffectiveContainerConfigSet {
        overlay_withheld: true,
        ..set(configs, vec![diagnostic])
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

const UNTRUSTED: &str = "repo-local config overlay /repo/.quecto/config.json is not trusted (sha256 abc) and was not applied; review it, then run `quecto config trust` from this directory";

#[test]
fn container_true_is_refused_while_the_checkouts_overlay_is_withheld() {
    // An overlay that was not applied may label another default: an
    // implicit `container: true` must not quietly land in the global one.
    let (select, _) = use_case(Ok(withheld(
        vec![entry("global", true), entry("other", false)],
        UNTRUSTED,
    )));
    let err = select
        .execute(&request(ContainerConfigSource::LaunchingAgent, None))
        .unwrap_err();
    assert_eq!(
        err,
        SelectContainerConfigError::OverlayWithheld {
            diagnostics: vec![UNTRUSTED.to_string()],
        }
    );
    assert_eq!(
        err.to_string(),
        format!(
            "container: true refused: the checkout's repo-local config overlay was not applied, so the container config it labels default is unknown ({UNTRUSTED}); trust it, or name a container_config explicitly to launch from the global configuration"
        )
    );
}

#[test]
fn an_explicit_name_launches_from_the_global_set_with_the_withheld_overlay_reported() {
    let (select, _) = use_case(Ok(withheld(
        vec![entry("global", true), entry("other", false)],
        UNTRUSTED,
    )));
    let selected = select
        .execute(&request(
            ContainerConfigSource::LaunchingAgent,
            Some("other"),
        ))
        .unwrap();
    assert_eq!(selected.config.name, "other");
    assert_eq!(selected.diagnostics, vec![UNTRUSTED.to_string()]);
    // A name the global set lacks still enumerates the global menu — and
    // names the withheld overlay, the likely reason the name is missing.
    let err = select
        .execute(&request(
            ContainerConfigSource::LaunchingAgent,
            Some("repo-only"),
        ))
        .unwrap_err();
    assert_eq!(
        err,
        SelectContainerConfigError::Unknown {
            name: "repo-only".into(),
            available: vec!["global".into(), "other".into()],
            diagnostics: vec![UNTRUSTED.into()],
        }
    );
    assert_eq!(
        err.to_string(),
        format!(
            "unknown container config 'repo-only' (available container configs: global, other); Configuration diagnostics: {UNTRUSTED}"
        )
    );
    assert_eq!(err.diagnostics(), [UNTRUSTED.to_string()]);
}

#[test]
fn a_no_default_refusal_carries_the_layer_diagnostics() {
    // A retired-local-file warning (no withheld overlay) rides the
    // no-default refusal too, one `Configuration diagnostics:` per line.
    let (select, _) = use_case(Ok(set(
        vec![entry("alpha", false)],
        vec!["warning: legacy", "second line"],
    )));
    let err = select
        .execute(&request(ContainerConfigSource::LaunchingAgent, None))
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "no container config is labeled \"default\": true (available container configs: alpha); Configuration diagnostics: warning: legacy; Configuration diagnostics: second line"
    );
    assert_eq!(
        err,
        SelectContainerConfigError::NoDefault {
            available: vec!["alpha".into()],
            diagnostics: vec!["warning: legacy".into(), "second line".into()],
        }
    );
}

#[test]
fn a_legacy_warning_alone_never_refuses_container_true() {
    // Diagnostics without a withheld overlay (the retired-local-file
    // warning) travel with the selection and refuse nothing.
    let (select, _) = use_case(Ok(set(
        vec![entry("global", true)],
        vec!["warning: /repo/config.json is no longer loaded"],
    )));
    let selected = select
        .execute(&request(ContainerConfigSource::LaunchingAgent, None))
        .unwrap();
    assert_eq!(selected.config.name, "global");
    assert_eq!(selected.diagnostics.len(), 1);
}
