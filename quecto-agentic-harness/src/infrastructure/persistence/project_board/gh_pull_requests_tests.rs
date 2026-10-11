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
         8) head -c 1000000 /dev/zero | tr '\\0' x;;\n\
         9) printf '\\033[31mfailed https://user:hunter2@github.com/x\\033[0m\\n' >&2; exit 1;;\n\
         10) (sleep 2; touch '{marker}.held') & echo OPEN;;\n\
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
        ("GH_HOST".into(), "evil.example".into()),
    ];
    (dir, vars)
}

#[test]
fn each_state_gh_reads_is_answered_and_a_missing_number_is_unknown() {
    let (dir, vars) = fake_gh();
    let host = GhPullRequestStates::new("github.com/platform-q-ai/quecto", vars).unwrap();
    assert_eq!(host.state(1), Ok(PullRequestState::Open));
    assert_eq!(host.state(2), Ok(PullRequestState::Merged));
    assert_eq!(host.state(3), Ok(PullRequestState::Closed));
    assert_eq!(host.state(4), Ok(PullRequestState::Unknown));
    let args = std::fs::read_to_string(dir.path().join("log.args")).unwrap();
    assert_eq!(
        args.trim(),
        "pr view 4 --repo github.com/platform-q-ai/quecto --json state --jq .state"
    );
    let env = std::fs::read_to_string(dir.path().join("log.env")).unwrap();
    for set in ["GH_TOKEN=token", "GH_PROMPT_DISABLED=1"] {
        assert!(env.lines().any(|line| line == set), "{set} missing: {env}");
    }
    for dropped in ["AWS_SECRET_ACCESS_KEY", "GIT_DIR", "GH_HOST"] {
        assert!(!env.contains(dropped), "{dropped} reached gh");
    }
}

#[test]
fn a_refusal_or_an_answer_gh_never_gives_is_unavailable_not_a_state() {
    let (_dir, vars) = fake_gh();
    let host = GhPullRequestStates::new("github.com/platform-q-ai/quecto", vars).unwrap();
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
    let host = GhPullRequestStates::new("github.com/platform-q-ai/quecto", vars)
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
    for repo in [
        "platform-q-ai/quecto",
        "github.com/a/b/c",
        "github.com/../x",
        "github.com/a/.",
        "github.com/a/..",
        "github.com/a/-x",
        "github.com/-o/x",
        "github.com/a b/c",
        "localhost/a/b",
        "",
    ] {
        assert!(
            GhPullRequestStates::new(repo, vars.clone()).is_err(),
            "{repo}"
        );
    }
    let no_gh = vec![("PATH".into(), "/nonexistent".into())];
    assert!(GhPullRequestStates::new("github.com/a/b", no_gh).is_err());
}

#[test]
fn gh_output_is_bounded_and_its_errors_sanitised() {
    let (_dir, vars) = fake_gh();
    let host = GhPullRequestStates::new("github.com/platform-q-ai/quecto", vars).unwrap();
    let flood = host.state(8);
    assert!(
        matches!(&flood, Err(PullRequestStatesError::Unavailable(why)) if why.len() < 2 * STDOUT_CAPACITY),
        "{flood:?}"
    );
    let Err(PullRequestStatesError::Unavailable(why)) = host.state(9) else {
        panic!("unavailable")
    };
    assert!(
        !why.contains('\u{1b}') && !why.contains("hunter2"),
        "{why:?}"
    );
    assert!(why.contains("failed"), "{why:?}");
}

#[test]
fn a_child_gh_leaves_holding_its_output_costs_the_grace_and_ends() {
    let (dir, vars) = fake_gh();
    let host = GhPullRequestStates::new("github.com/platform-q-ai/quecto", vars).unwrap();
    let started = Instant::now();
    assert_eq!(host.state(10), Ok(PullRequestState::Open));
    assert!(
        started.elapsed() < OUTPUT_GRACE * 4,
        "{:?}",
        started.elapsed()
    );
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        !dir.path().join("child-lived.held").exists(),
        "gh's child outlived it"
    );
}

#[test]
fn a_name_may_start_with_a_dot_and_enterprise_tokens_reach_only_their_configured_host() {
    let (dir, mut vars) = fake_gh();
    assert!(GhPullRequestStates::new("github.com/platform-q-ai/.github", vars.clone()).is_ok());
    assert!(
        GhPullRequestStates::new("ghe.example.com/ada_corp/repo", vars.clone()).is_ok(),
        "an enterprise-managed user's owner name"
    );
    vars.push(("GH_ENTERPRISE_TOKEN".into(), "enterprise".into()));
    let read_env = |repo: &str, vars: Vec<(OsString, OsString)>| {
        let host = GhPullRequestStates::new(repo, vars).unwrap();
        let _ = host.state(1);
        std::fs::read_to_string(dir.path().join("log.env")).unwrap()
    };
    assert!(!read_env("github.com/a/b", vars.clone()).contains("GH_ENTERPRISE_TOKEN"));
    assert!(
        !read_env("ghe.example.com/a/b", vars.clone()).contains("GH_ENTERPRISE_TOKEN"),
        "not configured"
    );
    let mut public = vars.clone();
    public.retain(|(key, _)| key != "GH_HOST");
    public.push(("GH_HOST".into(), "GitHub.com".into()));
    assert!(
        !read_env("github.com/a/b", public).contains("GH_ENTERPRISE_TOKEN"),
        "github.com never takes an enterprise token, even when named"
    );
    let mut configured = vars.clone();
    configured.retain(|(key, _)| key != "GH_HOST");
    configured.push(("GH_HOST".into(), "GHE.example.com".into()));
    let env = read_env("ghe.example.com/a/b", configured);
    assert!(env.contains("GH_ENTERPRISE_TOKEN=enterprise"), "{env}");
    assert!(
        !env.contains("GH_HOST"),
        "the host travels with the repository"
    );
}

#[test]
fn a_github_remote_names_its_repository_without_its_user_and_any_other_host_none() {
    for remote in [
        "https://github.com/platform-q-ai/quecto.git",
        "https://github.com/platform-q-ai/quecto",
        "https://user:token@github.com/platform-q-ai/quecto.git",
        "git@github.com:platform-q-ai/quecto.git",
        "ssh://git@GitHub.com/platform-q-ai/quecto.git",
    ] {
        assert_eq!(
            github_repository(remote, None).as_deref(),
            Some("github.com/platform-q-ai/quecto"),
            "{remote}"
        );
    }
    assert_eq!(
        github_repository("https://ghe.example.com/a/b.git", Some("GHE.example.com")).as_deref(),
        Some("ghe.example.com/a/b"),
        "an enterprise host when configured"
    );
    for remote in [
        "https://ghe.example.com/a/b.git",
        "https://gitlab.com/group/repo.git",
        "https://github.com/a/b/c",
        "https://github.com/../x",
        "http://github.com/a/b",
        "/srv/repo.git",
    ] {
        assert_eq!(github_repository(remote, None), None, "{remote}");
    }
}

/// Set for the test binary run as the parent that is killed: the `PATH`
/// its gh runs with.
#[cfg(target_os = "linux")]
const KILLED_PARENT: &str = "QUECTO_TEST_KILLED_PARENT_GH_PATH";

/// As the parent the next test kills: gh through the runner, hanging.
#[cfg(target_os = "linux")]
#[test]
fn a_parent_that_runs_a_hanging_gh() {
    let Some(path) = std::env::var_os(KILLED_PARENT) else {
        return;
    };
    let vars = [("PATH".into(), path), ("GH_TOKEN".into(), "token".into())];
    let host = GhPullRequestStates::new("github.com/platform-q-ai/quecto", vars).unwrap();
    let _ = host.state(1);
}

/// When quecto is killed outright, the gh it was running is asked to stop
/// with SIGTERM, its own session notwithstanding.
#[cfg(target_os = "linux")]
#[test]
fn a_gh_is_terminated_when_the_quecto_running_it_is_killed() {
    use crate::infrastructure::processes::parent_death_signal::tests::KilledParent;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let (pid, signal) = (dir.path().join("pid"), dir.path().join("signal"));
    let gh = bin.join("gh");
    let script = format!(
        "#!/bin/sh\ntrap 'echo TERM > \"{}\"; kill $!; exit 0' TERM\necho $$ > '{}'\nsleep 60 &\nwait\n",
        signal.display(),
        pid.display()
    );
    std::fs::write(&gh, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let name = "infrastructure::persistence::project_board::gh_pull_requests::tests::a_parent_that_runs_a_hanging_gh";
    let mut parent = KilledParent::start(name, &[(KILLED_PARENT, path.as_os_str())], &[&pid]);
    assert_eq!(
        parent.kill(Duration::from_secs(5)),
        [true],
        "gh outlived quecto"
    );
    assert_eq!(
        std::fs::read_to_string(&signal).ok().as_deref(),
        Some("TERM\n")
    );
}
