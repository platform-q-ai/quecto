//! The git runner: its environment, its session, and its one deadline for
//! the process and its output.
use super::*;

#[test]
fn a_git_that_hangs_is_stopped_with_its_whole_process_group() {
    let repos = Repos::new();
    let bin = repos.dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let marker = repos.dir.path().join("grandchild-lived");
    executable(
        &bin.join("git"),
        &format!(
            "#!/bin/sh\n(sleep 1; touch '{}') &\nexec sleep 30\n",
            marker.display()
        ),
    );
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let env = GitEnvironment::allowlisted([("PATH".into(), path)]);
    let mut git = Git::new(&repos.alice, env, person("Ada")).unwrap();
    git.network_timeout = std::time::Duration::from_millis(200);
    let started = std::time::Instant::now();
    let read = GitBoardStore::new(git, "origin").unwrap().read();
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("TimedOut")),
        "{read:?}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!marker.exists(), "the timed-out git's child outlived it");
}

/// git for `repos.alice` as `script`, with `vars` beside `PATH`.
fn shim(repos: &Repos, script: &str, vars: &[(&str, &str)]) -> Git {
    let bin = repos.dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("git"), script);
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let mut env: Vec<(OsString, OsString)> = vec![("PATH".into(), path)];
    env.extend(
        vars.iter()
            .map(|(key, value)| ((*key).into(), (*value).into())),
    );
    Git::new(
        &repos.alice,
        GitEnvironment::allowlisted(env),
        person("Ada"),
    )
    .unwrap()
}

#[test]
fn only_allowlisted_variables_reach_git() {
    let repos = Repos::new();
    let passed = [
        "HOME",
        "GIT_SSH_COMMAND",
        "GH_TOKEN",
        "HTTPS_PROXY",
        "no_proxy",
        "SSL_CERT_FILE",
    ];
    let dropped = [
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_WORK_TREE",
        "GIT_CONFIG_PARAMETERS",
        "AWS_SECRET_ACCESS_KEY",
        "OPENAI_API_KEY",
    ];
    let vars: Vec<(&str, &str)> = passed
        .iter()
        .chain(&dropped)
        .map(|key| (*key, "x"))
        .collect();
    let git = shim(&repos, "#!/bin/sh\nenv\n", &vars);
    let printed = git.text(&["anything"], None).unwrap();
    let keys: BTreeSet<&str> = printed
        .lines()
        .filter_map(|row| row.split_once('=').map(|(key, _)| key))
        .collect();
    for key in passed
        .iter()
        .chain(&["PATH", "GIT_TERMINAL_PROMPT", "LC_ALL"])
    {
        assert!(keys.contains(key), "{key} missing from {keys:?}");
    }
    for key in dropped {
        assert!(!keys.contains(key), "{key} reached git");
    }
}

#[test]
fn git_runs_in_a_session_of_its_own() {
    let repos = Repos::new();
    let git = shim(
        &repos,
        "#!/bin/sh\nread -r pid _ _ _ group session _ < /proc/$$/stat\necho \"$pid $group $session\"\n",
        &[],
    );
    let printed = git.text(&["anything"], None).unwrap();
    let ids: Vec<&str> = printed.split(' ').collect();
    assert_eq!(
        ids, [ids[0]; 3],
        "pid, process group and session are one: {printed}"
    );
}

#[test]
fn a_git_whose_child_keeps_its_output_open_ends_with_it() {
    let repos = Repos::new();
    let marker = repos.dir.path().join("child-lived");
    let script = format!(
        "#!/bin/sh\n(sleep 1; touch '{}') &\necho done\n",
        marker.display()
    );
    let git = shim(&repos, &script, &[]);
    let started = std::time::Instant::now();
    assert_eq!(git.text(&["anything"], None), Ok("done".to_string()));
    assert!(
        started.elapsed() < std::time::Duration::from_millis(900),
        "waited on the child's output"
    );
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!marker.exists(), "git's child outlived it");
}

#[test]
fn identities_git_would_alter_are_refused() {
    let repos = Repos::new();
    let env = || GitEnvironment::allowlisted(vars(&repos.alice, &[]));
    let as_named = |name: &str, email: &str| {
        let identity = Identity {
            name: name.into(),
            email: email.into(),
        };
        Git::new(&repos.alice, env(), identity).map(|_| ())
    };
    for (name, email) in [
        ("Ada Jr.", "ada@x.io"),
        ("Ada,", "ada@x.io"),
        ("'Ada'", "ada@x.io"),
        ("Ada", "ada@x.io."),
        ("Ada", "\"ada\"@x.io"),
    ] {
        assert!(
            matches!(as_named(name, email), Err(GitError::Identity(_))),
            "{name} <{email}>"
        );
    }
    for (name, email) in [
        ("Ada", "ada@x.io"),
        ("dependabot[bot]", "bot@x.io"),
        ("Shane (work)", "s@x.io"),
        ("Zoë", "z@x.io"),
    ] {
        assert_eq!(as_named(name, email), Ok(()), "{name} <{email}>");
    }
}
