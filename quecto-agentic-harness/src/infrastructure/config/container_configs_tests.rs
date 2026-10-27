use super::repository_from_argv;
use crate::application::subagents::dto::container_config::ConfigRepository;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

#[test]
fn the_repo_is_the_value_after_repo_before_the_child_separator() {
    assert_eq!(
        repository_from_argv(&argv(&[
            "/c",
            "--state-dir",
            "/s",
            "--repo",
            "https://r",
            "--"
        ])),
        ConfigRepository::Named("https://r".to_string())
    );
    // The shipped scripts read `--repo <url>` only.
    assert_eq!(
        repository_from_argv(&argv(&["/c", "--repo=https://r"])),
        ConfigRepository::Unnamed
    );
    // A repo named only after `--` belongs to the child command.
    assert_eq!(
        repository_from_argv(&argv(&["/c", "--", "quecto", "--repo", "https://r"])),
        ConfigRepository::Unnamed
    );
    for unnamed in [&["/c", "--repo"][..], &["/c", "--repo", ""], &["/c"]] {
        assert_eq!(
            repository_from_argv(&argv(unnamed)),
            ConfigRepository::Unnamed,
            "{unnamed:?}"
        );
    }
}

/// swarm-container-matches-repo: `create.sh` takes the last `--repo`, an
/// earlier reader the first; more than one before `--` is named as
/// repeated, never as either, so a swarm launch cannot trust it.
#[test]
fn more_than_one_repo_before_the_separator_is_repeated() {
    assert_eq!(
        repository_from_argv(&argv(&[
            "/c",
            "--repo",
            "https://a/o/r",
            "--image",
            "i",
            "--repo",
            "https://b/o/r",
            "--",
            "quecto",
            "--repo",
            "https://c/o/r",
        ])),
        ConfigRepository::Repeated(vec!["https://a/o/r".into(), "https://b/o/r".into()])
    );
    // A repeated `--repo` with a missing or empty value is still repeated.
    assert_eq!(
        repository_from_argv(&argv(&["/c", "--repo", "", "--repo", "https://b/o/r"])),
        ConfigRepository::Repeated(vec![String::new(), "https://b/o/r".into()])
    );
    assert_eq!(
        repository_from_argv(&argv(&["/c", "--repo", "https://a/o/r", "--repo"])),
        ConfigRepository::Repeated(vec!["https://a/o/r".into(), String::new()])
    );
    assert_eq!(
        ConfigRepository::Repeated(vec!["a".into(), "b".into()]).named(),
        None
    );
    assert_eq!(
        ConfigRepository::Repeated(vec!["a".into(), "b".into()]).shown(),
        Some("a, b".into())
    );
}

#[test]
fn the_adapter_says_whether_a_launching_agent_is_composed_without_naming_its_loaders() {
    use std::sync::Arc;

    use super::{ContainerConfigsFromEffectiveConfig, ExplicitConfigLoader};
    let explicit: ExplicitConfigLoader = Arc::new(|_| Err("never asked".to_string()));
    let without = ContainerConfigsFromEffectiveConfig::new(None, explicit.clone());
    assert_eq!(
        format!("{without:?}"),
        "ContainerConfigsFromEffectiveConfig { launching_agent: false, .. }"
    );
    let with = ContainerConfigsFromEffectiveConfig::new(
        Some(Arc::new(|| Err("never asked".to_string()))),
        explicit,
    );
    assert_eq!(
        format!("{with:?}"),
        "ContainerConfigsFromEffectiveConfig { launching_agent: true, .. }"
    );
}
