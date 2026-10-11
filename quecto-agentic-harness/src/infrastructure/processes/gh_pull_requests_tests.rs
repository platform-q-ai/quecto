use super::*;

/// A fake `gh` in a fresh directory: pull request 1 is open, 2 merged, 3
/// closed, 4 unknown to the repository, 5 refused for lack of a login, 6
/// never answers (and leaves a child that would touch `marker`), 7 an
/// answer gh never gives. It records its arguments and environment.
fn fake_gh() -> (tempfile::TempDir, Vec<(OsString, OsString)>) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let log = dir.path().join("log");
    let marker = dir.path().join("child-lived");
    let script = format!(
        "#!/bin/sh\necho \"$*\" > '{log}.args'\nenv > '{log}.env'\n\
         case \"$3\" in\n\
         1) echo OPEN;;\n2) echo MERGED;;\n3) echo CLOSED;;\n\
         4) echo 'GraphQL: Could not resolve to a PullRequest with the number of 4. (repository.pullRequest)' >&2; exit 1;;\n\
         5) echo 'To get started with GitHub CLI, please run:  gh auth login' >&2; exit 4;;\n\
         6) (sleep 1; touch '{marker}') & sleep 30;;\n\
         7) echo DRAFTED;;\n\
         esac\n",
        log = log.display(),
        marker = marker.display()
    );
    let gh = bin.join("gh");
    std::fs::write(&gh, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let vars = vec![
        ("PATH".into(), path),
        ("HOME".into(), dir.path().into()),
        ("GH_TOKEN".into(), "token".into()),
        ("AWS_SECRET_ACCESS_KEY".into(), "secret".into()),
        ("GIT_DIR".into(), "/elsewhere".into()),
    ];
    (dir, vars)
}

#[test]
fn each_state_gh_reads_is_answered_and_a_missing_number_is_unknown() {
    let (dir, vars) = fake_gh();
    let host = GhPullRequestStates::new("platform-q-ai/quecto", vars).unwrap();
    assert_eq!(host.state(1), Ok(PullRequestState::Open));
    assert_eq!(host.state(2), Ok(PullRequestState::Merged));
    assert_eq!(host.state(3), Ok(PullRequestState::Closed));
    assert_eq!(host.state(4), Ok(PullRequestState::Unknown));
    let args = std::fs::read_to_string(dir.path().join("log.args")).unwrap();
    assert_eq!(
        args.trim(),
        "pr view 4 --repo platform-q-ai/quecto --json state --jq .state"
    );
    let env = std::fs::read_to_string(dir.path().join("log.env")).unwrap();
    for set in ["GH_TOKEN=token", "GH_PROMPT_DISABLED=1"] {
        assert!(env.lines().any(|line| line == set), "{set} missing: {env}");
    }
    for dropped in ["AWS_SECRET_ACCESS_KEY", "GIT_DIR"] {
        assert!(!env.contains(dropped), "{dropped} reached gh");
    }
}

#[test]
fn a_refusal_or_an_answer_gh_never_gives_is_unavailable_not_a_state() {
    let (_dir, vars) = fake_gh();
    let host = GhPullRequestStates::new("platform-q-ai/quecto", vars).unwrap();
    let refused = host.state(5);
    assert!(
        matches!(&refused, Err(PullRequestStatesError::Unavailable(why)) if why.contains("gh auth login")),
        "{refused:?}"
    );
    assert!(matches!(
        host.state(7),
        Err(PullRequestStatesError::Unavailable(_))
    ));
}

#[test]
fn a_gh_that_hangs_is_ended_with_its_group_at_the_deadline() {
    let (dir, vars) = fake_gh();
    let host = GhPullRequestStates::new("platform-q-ai/quecto", vars)
        .unwrap()
        .with_timeout(Duration::from_millis(300));
    let started = Instant::now();
    assert!(matches!(
        host.state(6),
        Err(PullRequestStatesError::Unavailable(_))
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !dir.path().join("child-lived").exists(),
        "gh's child outlived it"
    );
}

#[test]
fn a_repository_is_owner_slash_name_and_gh_must_be_on_the_path() {
    let (_dir, vars) = fake_gh();
    for repo in ["quecto", "a/b/c", "../x", "a/.git", "-o/x", "a b/c", ""] {
        assert!(
            GhPullRequestStates::new(repo, vars.clone()).is_err(),
            "{repo}"
        );
    }
    let no_gh = vec![("PATH".into(), "/nonexistent".into())];
    assert!(GhPullRequestStates::new("a/b", no_gh).is_err());
}
