use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::ConfirmContainerRepository;
use crate::application::subagents::dto::container_config::ContainerLaunchConfig;
use crate::application::subagents::dto::container_repository::{
    ContainerRepository, ContainerRepositoryRefusal, ProjectRepository,
};
use crate::application::subagents::ports::container_repository::{ProjectOrigin, RepositoryUrls};
use crate::domain::environments::entities::repository_identity::RepositoryIdentity;

const CHECKOUT: &str = "/work/quecto-pocket";
const POCKET: &str = "https://github.com/platform-q-ai/quecto-pocket";
const QUECTO: &str = "https://github.com/platform-q-ai/quecto";

/// The checkout's origin as git would answer, recording each checkout asked.
struct FakeOrigin {
    answer: Result<Option<String>, String>,
    asked: Mutex<Vec<PathBuf>>,
}

impl ProjectOrigin for FakeOrigin {
    fn origin(&self, checkout: &Path) -> Result<Option<String>, String> {
        self.asked.lock().unwrap().push(checkout.to_path_buf());
        self.answer.clone()
    }
}

/// Reads exactly `https://github.com/<owner>/<name>`, nothing else: the
/// URL forms are infrastructure's, not this use case's.
struct FakeUrls;

impl RepositoryUrls for FakeUrls {
    fn identity(&self, url: &str) -> Option<RepositoryIdentity> {
        let path = url.strip_prefix("https://github.com/")?;
        let (owner, name) = path.split_once('/')?;
        RepositoryIdentity::new("github.com", owner, name)
    }
}

fn identity(owner: &str, name: &str) -> RepositoryIdentity {
    RepositoryIdentity::new("github.com", owner, name).unwrap()
}

fn config(name: &str, repository: Option<&str>) -> ContainerLaunchConfig {
    let mut create = vec!["/bin/create".to_string()];
    if let Some(url) = repository {
        create.extend(["--repo".to_string(), url.to_string()]);
    }
    ContainerLaunchConfig {
        name: name.to_string(),
        default: true,
        create,
        cleanup: vec!["/bin/cleanup".into()],
        exec: vec![],
        kill: vec![],
        inspect: vec![],
        repo_bound: false,
        repository: repository.map(String::from),
    }
}

fn confirm(
    checkout: Option<&str>,
    origin: Result<Option<&str>, &str>,
) -> (ConfirmContainerRepository, Arc<FakeOrigin>) {
    let origins = Arc::new(FakeOrigin {
        answer: origin
            .map(|url| url.map(String::from))
            .map_err(String::from),
        asked: Mutex::new(Vec::new()),
    });
    let use_case = ConfirmContainerRepository::new(
        checkout.map(PathBuf::from),
        origins.clone(),
        Arc::new(FakeUrls),
    );
    (use_case, origins)
}

fn refusal(
    result: Result<RepositoryIdentity, Box<ContainerRepositoryRefusal>>,
) -> ContainerRepositoryRefusal {
    *result.expect_err("the launch must be refused")
}

#[test]
fn a_global_default_for_another_repository_is_refused_naming_both_and_the_setup() {
    // The edinburgh incident: a quecto-pocket project with no container of
    // its own falls back to the global default, which clones quecto.
    let (use_case, origins) = confirm(Some(CHECKOUT), Ok(Some(POCKET)));
    let refused = refusal(use_case.confirm(&config("default", Some(QUECTO))));
    assert_eq!(
        refused,
        ContainerRepositoryRefusal {
            config: "default".into(),
            container: ContainerRepository::Named(identity("platform-q-ai", "quecto")),
            project: ProjectRepository::Known {
                checkout: PathBuf::from(CHECKOUT),
                identity: identity("platform-q-ai", "quecto-pocket"),
            },
        }
    );
    assert_eq!(
        refused.to_string(),
        "swarm launch refused before anything was created: this project (/work/quecto-pocket) is repository platform-q-ai/quecto-pocket, but container config 'default' clones platform-q-ai/quecto. A swarm runs only in a container of its own repository: set this repository's container up with `quecto container init --project /work/quecto-pocket --repo https://github.com/platform-q-ai/quecto-pocket`, then start again."
    );
    assert_eq!(
        *origins.asked.lock().unwrap(),
        vec![PathBuf::from(CHECKOUT)]
    );
}

#[test]
fn a_container_of_the_projects_own_repository_is_confirmed() {
    let (use_case, _) = confirm(Some(CHECKOUT), Ok(Some(POCKET)));
    assert_eq!(
        use_case.confirm(&config("standard", Some(POCKET))),
        Ok(identity("platform-q-ai", "quecto-pocket"))
    );
}

#[test]
fn a_global_default_naming_the_projects_repository_is_confirmed_whatever_its_case() {
    let (use_case, _) = confirm(Some(CHECKOUT), Ok(Some(POCKET)));
    let default = config(
        "default",
        Some("https://github.com/Platform-Q-AI/Quecto-Pocket"),
    );
    assert_eq!(
        use_case.confirm(&default),
        Ok(identity("platform-q-ai", "quecto-pocket"))
    );
}

#[test]
fn a_config_naming_no_repository_is_refused_as_unconfirmable() {
    let (use_case, _) = confirm(Some(CHECKOUT), Ok(Some(POCKET)));
    let refused = refusal(use_case.confirm(&config("sandbox", None)));
    assert_eq!(refused.container, ContainerRepository::Unnamed);
    assert_eq!(
        refused.to_string(),
        "swarm launch refused before anything was created: this project (/work/quecto-pocket) is repository platform-q-ai/quecto-pocket, but container config 'sandbox' names no repository (its create command has no --repo). quecto cannot confirm that container clones this repository: set this repository's container up with `quecto container init --project /work/quecto-pocket --repo https://github.com/platform-q-ai/quecto-pocket`, then start again."
    );
}

#[test]
fn a_config_whose_repository_cannot_be_read_is_refused_its_credential_redacted() {
    let (use_case, _) = confirm(Some(CHECKOUT), Ok(Some(POCKET)));
    let url = "https://ghp_secret@example.test/a/b/c";
    let refused = refusal(use_case.confirm(&config("odd", Some(url))));
    assert_eq!(
        refused.container,
        ContainerRepository::Unreadable(url.into())
    );
    let text = refused.to_string();
    assert!(!text.contains("ghp_secret"), "{text}");
    assert!(
        text.contains("container config 'odd' clones https://***@example.test/a/b/c, which quecto cannot read as a repository"),
        "{text}"
    );
}

#[test]
fn a_project_with_no_origin_is_refused_as_unconfirmable() {
    let (use_case, _) = confirm(Some(CHECKOUT), Ok(None));
    let refused = refusal(use_case.confirm(&config("default", Some(QUECTO))));
    assert_eq!(
        refused.project,
        ProjectRepository::NoOrigin {
            checkout: PathBuf::from(CHECKOUT)
        }
    );
    assert_eq!(
        refused.to_string(),
        "swarm launch refused before anything was created: this project (/work/quecto-pocket) is not a git checkout with an origin remote, so its repository is unknown, but container config 'default' clones platform-q-ai/quecto. quecto cannot confirm that container clones the project's repository: start the swarm from a clone whose origin is its repository, with that repository's container set up by `quecto container init --project <dir> --repo <url>`."
    );
}

#[test]
fn a_project_whose_origin_cannot_be_read_as_a_repository_is_refused() {
    let (use_case, _) = confirm(Some(CHECKOUT), Ok(Some("/srv/git/pocket.git")));
    let refused = refusal(use_case.confirm(&config("default", Some(POCKET))));
    assert_eq!(
        refused.project,
        ProjectRepository::Unreadable {
            checkout: PathBuf::from(CHECKOUT),
            origin: "/srv/git/pocket.git".into(),
        }
    );
}

#[test]
fn a_project_whose_origin_git_cannot_read_is_refused() {
    let (use_case, _) = confirm(Some(CHECKOUT), Err("cannot run git: not found"));
    let refused = refusal(use_case.confirm(&config("default", Some(POCKET))));
    assert_eq!(
        refused.project,
        ProjectRepository::OriginUnavailable {
            checkout: PathBuf::from(CHECKOUT),
            reason: "cannot run git: not found".into(),
        }
    );
    assert!(
        refused
            .to_string()
            .contains("could not be read (cannot run git: not found)")
    );
}

#[test]
fn a_launcher_with_no_project_checkout_is_refused_without_asking_git() {
    let (use_case, origins) = confirm(None, Ok(Some(POCKET)));
    let refused = refusal(use_case.confirm(&config("default", Some(POCKET))));
    assert_eq!(refused.project, ProjectRepository::NoCheckout);
    assert!(origins.asked.lock().unwrap().is_empty());
}

#[test]
fn the_debug_form_shows_the_checkout_only() {
    let (use_case, _) = confirm(Some(CHECKOUT), Ok(None));
    assert_eq!(
        format!("{use_case:?}"),
        "ConfirmContainerRepository { checkout: Some(\"/work/quecto-pocket\"), .. }"
    );
}
