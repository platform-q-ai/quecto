//! A swarm's selection (swarm-container-matches-repo): the selection,
//! then the repository check.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::SelectContainerConfig;
use crate::application::subagents::dto::container_config::{
    ContainerConfigSource, ContainerConfigsError, ContainerLaunchConfig,
    EffectiveContainerConfigSet, SelectContainerConfigError, SelectContainerConfigRequest,
};
use crate::application::subagents::dto::container_repository::{
    ContainerRepository, ProjectRepository,
};
use crate::application::subagents::dto::standard_script::StandardScriptVerdict;
use crate::application::subagents::ports::container_configs::{
    ContainerScriptIntegrity, EffectiveContainerConfigs,
};
use crate::application::subagents::ports::container_repository::{ProjectOrigin, RepositoryUrls};
use crate::application::subagents::use_cases::ConfirmContainerRepository;
use crate::domain::environments::entities::repository_identity::RepositoryIdentity;

const CHECKOUT: &str = "/work/quecto-pocket";
const POCKET: &str = "https://github.com/platform-q-ai/quecto-pocket";
const QUECTO: &str = "https://github.com/platform-q-ai/quecto";

struct Configs(Vec<ContainerLaunchConfig>);

impl EffectiveContainerConfigs for Configs {
    fn effective_container_configs(
        &self,
        _: &ContainerConfigSource,
    ) -> Result<EffectiveContainerConfigSet, ContainerConfigsError> {
        Ok(EffectiveContainerConfigSet {
            configs: self.0.clone(),
            diagnostics: vec![],
            overlay_withheld: false,
        })
    }
}

struct NotStandard;

impl ContainerScriptIntegrity for NotStandard {
    fn verify(&self, _: &Path) -> StandardScriptVerdict {
        StandardScriptVerdict::NotStandard
    }
}

struct Origin(&'static str);

impl ProjectOrigin for Origin {
    fn origin(&self, _: &Path) -> Result<Option<String>, String> {
        Ok(Some(self.0.to_string()))
    }
}

struct GithubUrls;

impl RepositoryUrls for GithubUrls {
    fn identity(&self, url: &str) -> Option<RepositoryIdentity> {
        let (owner, name) = url.strip_prefix("https://github.com/")?.split_once('/')?;
        RepositoryIdentity::new("github.com", owner, name)
    }
}

fn entry(
    name: &str,
    default: bool,
    repo_bound: bool,
    repository: Option<&str>,
) -> ContainerLaunchConfig {
    ContainerLaunchConfig {
        name: name.to_string(),
        default,
        create: vec!["/bin/create".into()],
        cleanup: vec!["/bin/cleanup".into()],
        exec: vec![],
        kill: vec![],
        inspect: vec![],
        repo_bound,
        repository: repository.map_or(
            crate::application::subagents::dto::container_config::ConfigRepository::Unnamed,
            |url| {
                crate::application::subagents::dto::container_config::ConfigRepository::Named(
                    url.into(),
                )
            },
        ),
    }
}

fn selection(configs: Vec<ContainerLaunchConfig>) -> SelectContainerConfig {
    SelectContainerConfig::new(Arc::new(Configs(configs)), Arc::new(NotStandard))
}

fn confirming(configs: Vec<ContainerLaunchConfig>, origin: &'static str) -> SelectContainerConfig {
    let confirm = ConfirmContainerRepository::new(
        Some(PathBuf::from(CHECKOUT)),
        Arc::new(Origin(origin)),
        Arc::new(GithubUrls),
    );
    selection(configs).confirming(Arc::new(confirm))
}

fn implicit() -> SelectContainerConfigRequest {
    SelectContainerConfigRequest {
        source: ContainerConfigSource::LaunchingAgent,
        name: None,
    }
}

#[test]
fn a_project_without_its_own_config_is_refused_the_global_default_of_another_repository() {
    let select = confirming(vec![entry("default", true, false, Some(QUECTO))], POCKET);
    let refused = select.select_for_swarm(&implicit()).unwrap_err();
    let SelectContainerConfigError::Repository(refusal) = &refused else {
        panic!("a repository refusal, not {refused:?}");
    };
    assert_eq!(refusal.config(), "default");
    assert!(matches!(refusal.container(), ContainerRepository::Named(_)));
    assert!(matches!(refusal.project(), ProjectRepository::Known { .. }));
    assert_eq!(refused.diagnostics(), &[] as &[String]);
    let text = refused.to_string();
    assert!(
        text.starts_with("swarm launch refused before anything was created: "),
        "{text}"
    );
    assert!(
        text.contains("platform-q-ai/quecto-pocket")
            && text.contains("clones platform-q-ai/quecto."),
        "{text}"
    );
    assert!(
        text.contains("`quecto container init --project /work/quecto-pocket --repo https://github.com/platform-q-ai/quecto-pocket`"),
        "{text}"
    );
}

#[test]
fn a_global_default_naming_the_projects_repository_is_selected_for_a_swarm() {
    let select = confirming(vec![entry("default", true, false, Some(POCKET))], POCKET);
    let selected = select.select_for_swarm(&implicit()).unwrap();
    assert_eq!(selected.config.name, "default");
}

#[test]
fn the_projects_own_standard_config_is_selected_for_a_swarm_as_before() {
    let select = confirming(
        vec![
            entry("default", true, false, Some(QUECTO)),
            entry("standard", false, true, Some(POCKET)),
        ],
        POCKET,
    );
    let selected = select.select_for_swarm(&implicit()).unwrap();
    assert_eq!(selected.config.name, "standard");
    // An ordinary selection is unchanged by the swarm rule.
    assert_eq!(select.execute(&implicit()).unwrap(), selected);
}

#[test]
fn an_explicitly_named_config_of_another_repository_is_refused_for_a_swarm() {
    let select = confirming(
        vec![
            entry("standard", true, true, Some(POCKET)),
            entry("quecto", false, false, Some(QUECTO)),
        ],
        POCKET,
    );
    let named = SelectContainerConfigRequest {
        name: Some("quecto".into()),
        ..implicit()
    };
    assert!(matches!(
        select.select_for_swarm(&named),
        Err(SelectContainerConfigError::Repository(_))
    ));
}

#[test]
fn a_config_naming_no_repository_is_refused_for_a_swarm() {
    let select = confirming(vec![entry("sandbox", true, false, None)], POCKET);
    let SelectContainerConfigError::Repository(refusal) =
        select.select_for_swarm(&implicit()).unwrap_err()
    else {
        panic!("a repository refusal");
    };
    assert_eq!(*refusal.container(), ContainerRepository::Unnamed);
}

#[test]
fn a_selection_composed_without_a_repository_check_refuses_every_swarm() {
    let select = selection(vec![entry("default", true, false, Some(POCKET))]);
    let refused = select.select_for_swarm(&implicit()).unwrap_err();
    assert_eq!(
        refused,
        SelectContainerConfigError::NoRepositoryCheck {
            name: "default".into()
        }
    );
    assert_eq!(
        refused.to_string(),
        "swarm launch refused before anything was created: this launcher has no repository check, so quecto cannot confirm that container config 'default' clones the project's repository"
    );
}

#[test]
fn a_selection_refusal_comes_before_the_repository_check() {
    let select = confirming(vec![], POCKET);
    assert!(matches!(
        select.select_for_swarm(&implicit()),
        Err(SelectContainerConfigError::NoDefault { .. })
    ));
}
