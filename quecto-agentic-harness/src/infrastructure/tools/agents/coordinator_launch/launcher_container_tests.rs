//! swarm-container-matches-repo: a board start asks the launcher, before
//! it writes anything, whether the coordinator's container would be
//! refused, and the launcher answers with the selection spawn makes.
use std::path::{Path, PathBuf};

use super::*;
use crate::application::subagents::dto::container_config::{
    ContainerConfigsError, ContainerLaunchConfig, EffectiveContainerConfigSet,
};
use crate::application::subagents::dto::standard_script::StandardScriptVerdict;
use crate::application::subagents::ports::container_configs::{
    ContainerScriptIntegrity, EffectiveContainerConfigs,
};
use crate::application::subagents::ports::container_repository::ProjectOrigin;
use crate::application::subagents::use_cases::ConfirmContainerRepository;
use crate::domain::shared::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::ToolDefinition;
use crate::infrastructure::config::container_repository_urls::RemoteRepositoryUrls;

struct Configs(Vec<ContainerLaunchConfig>);

impl EffectiveContainerConfigs for Configs {
    fn effective_container_configs(
        &self,
        source: &ContainerConfigSource,
    ) -> Result<EffectiveContainerConfigSet, ContainerConfigsError> {
        assert_eq!(
            source,
            &ContainerConfigSource::LaunchingAgent,
            "spawn's own layers"
        );
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
        Ok(Some(self.0.into()))
    }
}

/// A tool no test may call: the judgement runs no spawn.
struct Uncallable;

impl Tool for Uncallable {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".into(),
            description: "never called".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>,
    > {
        panic!("judging a container never spawns");
    }
}

fn default_config(repository: &str) -> ContainerLaunchConfig {
    ContainerLaunchConfig {
        name: "default".into(),
        default: true,
        create: vec!["/bin/create".into(), "--repo".into(), repository.into()],
        cleanup: vec!["/bin/cleanup".into()],
        exec: vec![],
        kill: vec![],
        inspect: vec![],
        repo_bound: false,
        repository: crate::application::subagents::dto::container_config::ConfigRepository::Named(
            repository.into(),
        ),
    }
}

fn launcher(containers: Option<Arc<SelectContainerConfig>>) -> SpawnCoordinatorLauncher {
    SpawnCoordinatorLauncher::new(
        Arc::new(Uncallable),
        Arc::new(Uncallable),
        SpawnPermit::default(),
        super::super::subagent_registry::new_registry(),
    )
    .judging_containers(containers)
}

fn selection(default_repository: &str, origin: &'static str) -> Arc<SelectContainerConfig> {
    let confirm = ConfirmContainerRepository::new(
        Some(PathBuf::from("/work/quecto-pocket")),
        Arc::new(Origin(origin)),
        Arc::new(RemoteRepositoryUrls),
    );
    Arc::new(
        SelectContainerConfig::new(
            Arc::new(Configs(vec![default_config(default_repository)])),
            Arc::new(NotStandard),
        )
        .confirming(Arc::new(confirm)),
    )
}

#[test]
fn a_global_default_of_another_repository_is_refused_before_the_start_writes() {
    let launcher = launcher(Some(selection(
        "https://github.com/platform-q-ai/quecto",
        "git@github.com:platform-q-ai/quecto-pocket.git",
    )));
    let refusal = launcher.container_refusal().expect("refused");
    assert!(
        refusal.starts_with("swarm launch refused before anything was created: this project (/work/quecto-pocket) is repository platform-q-ai/quecto-pocket, but container config 'default' clones platform-q-ai/quecto."),
        "{refusal}"
    );
}

#[test]
fn a_default_of_the_projects_repository_in_another_spelling_is_not_refused() {
    let launcher = launcher(Some(selection(
        "https://github.com/Platform-Q-AI/quecto-pocket.git/",
        "git@github.com:platform-q-ai/quecto-pocket.git",
    )));
    assert_eq!(launcher.container_refusal(), None);
}

/// No selection: the repository cannot be confirmed, so it refuses.
#[test]
fn a_launcher_with_no_selection_refuses() {
    assert_eq!(
        launcher(None).container_refusal(),
        Some(super::NO_SELECTION.to_string())
    );
}

/// A selection fault that is not the repository's (here: no default at
/// all) is left to the launch, which refuses it as `launch_failed` as
/// before.
#[test]
fn a_selection_fault_other_than_the_repository_is_left_to_the_launch() {
    let confirm = ConfirmContainerRepository::new(
        Some(PathBuf::from("/work/quecto-pocket")),
        Arc::new(Origin("https://github.com/platform-q-ai/quecto-pocket")),
        Arc::new(RemoteRepositoryUrls),
    );
    let empty = SelectContainerConfig::new(Arc::new(Configs(vec![])), Arc::new(NotStandard))
        .confirming(Arc::new(confirm));
    assert_eq!(launcher(Some(Arc::new(empty))).container_refusal(), None);
}

/// A selection with no repository check is a repository refusal.
#[test]
fn a_selection_without_a_repository_check_refuses() {
    let unchecked = SelectContainerConfig::new(
        Arc::new(Configs(vec![default_config(
            "https://github.com/platform-q-ai/quecto-pocket",
        )])),
        Arc::new(NotStandard),
    );
    let refusal = launcher(Some(Arc::new(unchecked)))
        .container_refusal()
        .expect("refused");
    assert!(refusal.contains("has no repository check"), "{refusal}");
}
