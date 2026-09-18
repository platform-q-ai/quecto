use super::repository_from_argv;

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
        Some("https://r".to_string())
    );
    // The shipped scripts read `--repo <url>` only.
    assert_eq!(
        repository_from_argv(&argv(&["/c", "--repo=https://r"])),
        None
    );
    // A repo named only after `--` belongs to the child command.
    assert_eq!(
        repository_from_argv(&argv(&["/c", "--", "quecto", "--repo", "https://r"])),
        None
    );
    assert_eq!(repository_from_argv(&argv(&["/c", "--repo"])), None);
    assert_eq!(repository_from_argv(&argv(&["/c", "--repo", ""])), None);
    assert_eq!(repository_from_argv(&argv(&["/c"])), None);
}
