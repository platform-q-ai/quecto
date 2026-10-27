//! swarm-container-matches-repo: a coordinator's (a swarm's) new container
//! must clone the launching project's own repository; one that does not is
//! refused before its create script runs. Ordinary container children are
//! unchanged.
use super::tests::{base_config, test_supervisor};
use super::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A project checkout whose `origin` is `origin`.
fn checkout(root: &Path, origin: &str) -> PathBuf {
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    for args in [
        &["init", "-q"][..],
        &["remote", "add", "origin", origin][..],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&project)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    project
}

/// A config file whose one default entry clones `repository` with a create
/// script that leaves `marker` behind when it runs.
fn config_file(root: &Path, repository: &str, marker: &Path) -> PathBuf {
    let script = root.join("create.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\necho not-json\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = root.join("config.json");
    let config = serde_json::json!({"container_configs": {"default": {
        "default": true,
        "create": [script, "--repo", repository],
        "cleanup": ["/bin/true"],
    }}});
    std::fs::write(&path, config.to_string()).unwrap();
    path
}

/// A spawn of a new container from the project, as a coordinator when
/// `coordinator`, with the default entry of the file `config_path`.
async fn spawn(
    root: &Path,
    project: &Path,
    config_path: PathBuf,
    coordinator: bool,
) -> Result<PreparedChild, DomainError> {
    let selection = crate::composition::container_configs::build_project_container_config_handles(
        root, None, project,
    )
    .selection;
    let mut config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    config.config_path = Some(config_path);
    config.coordinator = coordinator;
    spawn_prepared_child(
        &config,
        &ChildCommand {
            swarm_context: None,
            swarm_member: false,
            supervisor: &test_supervisor(),
            binary: Path::new("true"),
            cli_args: &[],
            base_dir: root,
            admission_dir: None,
        },
        &EnvironmentRegistry::new(),
        Some(&selection),
    )
    .await
}

#[tokio::test]
async fn a_swarm_is_refused_a_container_of_another_repository_before_its_create_runs() {
    let root = TempDir::new().unwrap();
    let project = checkout(
        root.path(),
        "git@github.com:platform-q-ai/quecto-pocket.git",
    );
    let marker = root.path().join("created");
    let config = config_file(
        root.path(),
        "https://github.com/platform-q-ai/quecto",
        &marker,
    );
    let Err(error) = spawn(root.path(), &project, config, true).await else {
        panic!("refused");
    };
    let text = error.to_string();
    assert!(
        text.starts_with(
            "tool error: swarm launch refused before anything was created: this project ("
        ),
        "{text}"
    );
    assert!(
        text.contains("is repository platform-q-ai/quecto-pocket, but container config 'default' clones platform-q-ai/quecto."),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "`quecto container init --project {} --repo https://github.com/platform-q-ai/quecto-pocket`",
            project.display()
        )),
        "{text}"
    );
    assert!(!marker.exists(), "the create script never ran");
}

#[tokio::test]
async fn a_swarm_is_refused_a_container_naming_no_repository() {
    let root = TempDir::new().unwrap();
    let project = checkout(
        root.path(),
        "https://github.com/platform-q-ai/quecto-pocket",
    );
    let marker = root.path().join("created");
    let config = config_file(root.path(), "/srv/git/somewhere", &marker);
    let Err(error) = spawn(root.path(), &project, config, true).await else {
        panic!("refused");
    };
    assert!(
        error
            .to_string()
            .contains("which quecto cannot read as a repository"),
        "{error}"
    );
    assert!(!marker.exists(), "the create script never ran");
}

#[tokio::test]
async fn a_swarm_in_a_container_of_its_own_repository_creates_as_before() {
    let root = TempDir::new().unwrap();
    let project = checkout(
        root.path(),
        "git@github.com:platform-q-ai/quecto-pocket.git",
    );
    let marker = root.path().join("created");
    let config = config_file(
        root.path(),
        "https://github.com/Platform-Q-AI/quecto-pocket.git",
        &marker,
    );
    let Err(error) = spawn(root.path(), &project, config, true).await else {
        panic!("the stub's create answers no result");
    };
    assert!(
        !error.to_string().contains("swarm launch refused"),
        "{error}"
    );
    assert!(marker.exists(), "the create script ran");
}

#[tokio::test]
async fn an_ordinary_container_child_is_not_held_to_the_swarm_rule() {
    let root = TempDir::new().unwrap();
    let project = checkout(
        root.path(),
        "git@github.com:platform-q-ai/quecto-pocket.git",
    );
    let marker = root.path().join("created");
    let config = config_file(
        root.path(),
        "https://github.com/platform-q-ai/quecto",
        &marker,
    );
    let Err(error) = spawn(root.path(), &project, config, false).await else {
        panic!("the stub's create answers no result");
    };
    assert!(
        !error.to_string().contains("swarm launch refused"),
        "{error}"
    );
    assert!(marker.exists(), "the create script ran");
}
