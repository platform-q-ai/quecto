//! The git runner: its environment, its session, and its one deadline for
//! the process and its output.
use super::*;
use crate::infrastructure::persistence::project_board::git::SHORT_OUTPUT;

#[test]
fn a_git_that_hangs_is_stopped_with_its_whole_process_group() {
    let repos = Repos::new();
    let marker = repos.dir.path().join("grandchild-lived");
    let script = format!(
        "#!/bin/sh\n(sleep 1; touch '{}') &\nexec sleep 30\n",
        marker.display()
    );
    let mut git = shim(&repos, &script, &[]);
    git.network_timeout = std::time::Duration::from_millis(200);
    let started = std::time::Instant::now();
    let run = git.network(&["fetch"]).map(|_| ());
    assert!(matches!(&run, Err(GitError::TimedOut { .. })), "{run:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!marker.exists(), "the timed-out git's child outlived it");
}

/// The real git on `PATH`.
fn real_git() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|candidate| candidate.is_file())
        .unwrap()
}

/// git for `repos.alice` as `script`, with `vars` beside `PATH`. Asking
/// it for the committer identity reaches the real git, so it can be built.
fn shim(repos: &Repos, script: &str, vars: &[(&str, &str)]) -> Git {
    let bin = repos.dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let (shebang, body) = script.split_once('\n').unwrap();
    let script = format!(
        "{shebang}\ncase \" $* \" in *\" var GIT_COMMITTER_IDENT \"*) exec '{}' \"$@\";; esac\n{body}",
        real_git().display()
    );
    executable(&bin.join("git"), &script);
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
        "GITHUB_TOKEN",
        "GH_CONFIG_DIR",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "all_proxy",
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
    // A local call never fetches a missing object on its own (a partial
    // clone fetches what the board needs in one call instead).
    assert!(
        printed.lines().any(|row| row == "GIT_NO_LAZY_FETCH=1"),
        "{printed}"
    );
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

/// What git records for a committer named `name <email>`, by a real
/// `commit-tree` in a throwaway repository.
fn recorded_by_git(name: &str, email: &str) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    sh(dir.path(), &["init", "-q", "scratch"]);
    let scratch = dir.path().join("scratch");
    let tree = sh_with(&scratch, &["mktree"], b"");
    let output = Command::new("git")
        .current_dir(&scratch)
        .args(["commit-tree", &tree, "-m", "who"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .envs([("GIT_AUTHOR_NAME", name), ("GIT_AUTHOR_EMAIL", email)])
        .envs([("GIT_COMMITTER_NAME", name), ("GIT_COMMITTER_EMAIL", email)])
        .output()
        .unwrap();
    assert!(output.status.success(), "{name} <{email}>");
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let who = sh(&scratch, &["log", "-1", "--format=%cn%x00%ce", &commit]);
    let (name, email) = who.split_once('\0').unwrap();
    (name.to_string(), email.to_string())
}

/// Exactly the identities this machine's git would record as given are
/// taken: the table is judged by the real git, not by a copy of its rules.
#[test]
fn an_identity_is_taken_exactly_when_git_records_it_as_given() {
    let repos = Repos::new();
    let env = || GitEnvironment::allowlisted(vars(&repos.alice, &[]));
    let nfd = "Zoe\u{308}";
    let table = [
        ("Ada", "ada@x.io"),
        ("Ada Jr.", "ada@x.io"),
        ("Ada`", "ada@x.io"),
        ("Ada 🦀", "ada@x.io"),
        ("Ada…", "ada@x.io"),
        ("Ada»", "ada@x.io"),
        (nfd, "zoe@x.io"),
        ("dependabot[bot]", "bot@x.io"),
        ("Shane (work)", "s@x.io"),
        ("Ada", "ada@x.io."),
        ("Ada,", "ada@x.io"),
        ("'Ada'", "ada@x.io"),
        ("Ada;", "ada@x.io"),
        ("Ada:", "ada@x.io"),
        ("Ada\\", "ada@x.io"),
        ("\"Ada\"", "ada@x.io"),
        ("Ada", "\"ada\"@x.io"),
        ("Ada", "ada@x.io;"),
    ];
    let (mut kept, mut changed) = (0, 0);
    for (name, email) in table {
        let identity = Identity {
            name: name.into(),
            email: email.into(),
        };
        assert_eq!(identity.validate("identity"), Ok(()), "{name} <{email}>");
        let recorded = recorded_by_git(name, email);
        let as_given = recorded == (name.to_string(), email.to_string());
        let built = Git::new(&repos.alice, env(), identity).map(|_| ());
        match as_given {
            true => {
                kept += 1;
                assert_eq!(built, Ok(()), "{name} <{email}>");
            }
            false => {
                changed += 1;
                assert!(
                    matches!(&built, Err(GitError::Identity(_))),
                    "{name} <{email}> is recorded as {recorded:?}: {built:?}"
                );
            }
        }
    }
    assert!(kept > 0 && changed > 0, "{kept} kept, {changed} changed");
}

#[test]
fn output_past_its_bound_is_an_error_and_never_hangs() {
    let repos = Repos::new();
    let git = shim(&repos, "#!/bin/sh\nhead -c 4000000 /dev/zero\n", &[]);
    let started = std::time::Instant::now();
    let read = git.text(&["anything"], None);
    assert!(
        matches!(&read, Err(GitError::TooMuchOutput { limit, .. }) if *limit == SHORT_OUTPUT),
        "{:?}",
        read.map(|text| text.len())
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn stderr_is_bounded_and_stripped_of_control_characters() {
    let repos = Repos::new();
    let git = shim(
        &repos,
        "#!/bin/sh\nprintf '\\033[31mred\\007\\r\\nline\\n' >&2\nhead -c 200000 /dev/zero | tr '\\0' e >&2\nexit 1\n",
        &[],
    );
    let failed = git.ok(&["anything"], None);
    let Err(GitError::Failed { stderr, .. }) = failed else {
        panic!("{failed:?}");
    };
    assert!(stderr.len() <= 16 * 1024, "{} bytes", stderr.len());
    assert!(
        stderr.contains("red") && stderr.contains("line"),
        "{stderr:.80}"
    );
    assert!(!stderr.chars().any(char::is_control), "{stderr:.80}");
}

/// A git that exited has said all it will: its output gets a short grace
/// to drain, even at its deadline, so a git that succeeded is never timed
/// out because something outside its process group held its pipe.
#[test]
fn a_git_that_exited_is_not_timed_out_while_its_output_drains() {
    let repos = Repos::new();
    let mut git = shim(&repos, "#!/bin/sh\nsetsid sleep 0.6 &\necho done\n", &[]);
    git.local_timeout = std::time::Duration::from_millis(300);
    assert_eq!(git.text(&["anything"], None), Ok("done".to_string()));
}
