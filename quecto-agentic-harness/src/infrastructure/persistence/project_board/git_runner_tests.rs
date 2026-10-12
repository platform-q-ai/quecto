//! The git runner: its environment, its session, and its one deadline for
//! the process and its output.
use super::*;
use crate::infrastructure::persistence::project_board::git::{
    MAX_QUOTED_CHARS, MAX_STDERR, SHORT_OUTPUT, quoted, redacted_stderr, shown_name,
};
use crate::infrastructure::persistence::project_board::own_group::tests::ready_before_its_deadline;

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

/// A git that cannot be started says so, naming the session its spawn
/// makes first, and the death signal it arms where one is armed (Linux; a
/// refusal to arm fails the spawn too).
#[test]
fn a_git_that_cannot_start_names_what_its_spawn_arms() {
    let repos = Repos::new();
    let git = shim(&repos, "#!/bin/sh\nexit 0\n", &[]);
    std::fs::remove_file(repos.dir.path().join("bin").join("git")).unwrap();
    let run = git.network(&["fetch"]).map(|_| ());
    assert!(
        matches!(&run, Err(GitError::Spawn(why)) if why.contains("making its session")
            && why.contains("arming the death signal") == cfg!(target_os = "linux")),
        "{run:?}"
    );
}

/// At its deadline git is asked to stop with SIGTERM first, so it can
/// remove a ref lock as it goes. The stub says it is ready once its trap
/// is set, and only a run that saw that before its deadline counts.
#[test]
fn a_git_past_its_deadline_is_asked_to_stop_first() {
    let repos = Repos::new();
    let (ready, signal) = (
        repos.dir.path().join("ready"),
        repos.dir.path().join("signal"),
    );
    let script = format!(
        "#!/bin/sh\ntrap 'echo TERM > \"{}\"; exit 0' TERM\necho > '{}'\nsleep 30 &\nwait\n",
        signal.display(),
        ready.display()
    );
    let git = shim(&repos, &script, &[]);
    let (run, ()) = ready_before_its_deadline(
        &[&ready, &signal],
        || ready.exists().then_some(()),
        |deadline| {
            let mut git = git.clone();
            git.network_timeout = deadline;
            git.network(&["fetch"]).map(|_| ())
        },
    );
    assert!(matches!(&run, Err(GitError::TimedOut { .. })), "{run:?}");
    assert_eq!(
        std::fs::read_to_string(&signal).ok().as_deref(),
        Some("TERM\n")
    );
}

/// A git group that ignores SIGTERM is killed after a short grace, not
/// left running. The stub names its child once its trap is set, so the
/// child is watched by its start time while it still runs.
#[cfg(target_os = "linux")]
#[test]
fn a_git_that_ignores_the_request_is_killed_after_a_grace() {
    use crate::infrastructure::processes::parent_death_signal::tests::Watched;
    let repos = Repos::new();
    let pid = repos.dir.path().join("pid");
    let script = format!(
        "#!/bin/sh\ntrap '' TERM\nsleep 30 &\necho $! > '{}'\nwait\n",
        pid.display()
    );
    let git = shim(&repos, &script, &[]);
    let ((run, took, deadline), sleeper) = ready_before_its_deadline(
        &[&pid],
        || Watched::named_in(&pid),
        |deadline| {
            let mut git = git.clone();
            git.network_timeout = deadline;
            let started = std::time::Instant::now();
            let run = git.network(&["fetch"]).map(|_| ());
            (run, started.elapsed(), deadline)
        },
    );
    assert!(matches!(&run, Err(GitError::TimedOut { .. })), "{run:?}");
    assert!(
        took < deadline + std::time::Duration::from_secs(3),
        "{took:?}"
    );
    let by = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !sleeper.gone() && std::time::Instant::now() < by {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let gone = sleeper.gone();
    sleeper.end();
    assert!(gone, "the sleeper outlived git");
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
    let output = git.local(&["anything"], None, SHORT_OUTPUT).unwrap();
    // git printed more than is read, ending in one unbroken run the read
    // cut: that run goes (it could be a secret cut short), the lines
    // before it stay.
    assert!(output.stderr.len() <= MAX_STDERR);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim_end(),
        "\u{1b}[31mred\u{7}\r\nline"
    );
    let failed = git.ok(&["anything"], None);
    let Err(GitError::Failed { stderr, .. }) = failed else {
        panic!("{failed:?}");
    };
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

/// Only letters, marks, numbers, punctuation, symbols and spaces are
/// quoted; anything else (zero-width, byte-order and tag characters, any
/// control) is one space.
#[test]
fn quoted_text_keeps_only_visible_categories() {
    assert_eq!(
        quoted("a\u{200b}b\u{feff}c\u{e0041}d\u{2028}e\u{85}f".as_bytes()),
        "a b c d e f"
    );
    assert_eq!(
        quoted("Zoe\u{308} ✓ 1.5% «x»".as_bytes()),
        "Zoe\u{308} ✓ 1.5% «x»"
    );
}

/// Once git has exited, its output drains for a short grace, never for
/// the rest of its deadline, however long something outside its group
/// holds the pipe.
#[test]
fn a_git_that_exited_drains_its_output_for_the_grace_alone() {
    let repos = Repos::new();
    let git = shim(&repos, "#!/bin/sh\nsetsid sleep 6 &\necho done\n", &[]);
    assert_eq!(git.local_timeout, std::time::Duration::from_secs(30));
    let started = std::time::Instant::now();
    let read = git.text(&["anything"], None);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "{:?}: {read:?}",
        started.elapsed()
    );
}

/// Only what git says about the identity is an identity error; a git that
/// cannot run here says so.
#[test]
fn only_an_identity_git_refuses_is_an_identity_error() {
    let repos = Repos::new();
    let env = || GitEnvironment::allowlisted(vars(&repos.alice, &[]));
    let as_named = |repo: &Path, name: &str| {
        let identity = Identity {
            name: name.into(),
            email: "ada@x.io".into(),
        };
        Git::new(repo, env(), identity).map(|_| ())
    };
    let refused = as_named(&repos.alice, ",,");
    assert!(
        matches!(&refused, Err(GitError::Identity(why)) if why.contains("disallowed")),
        "{refused:?}"
    );
    let nowhere = repos.dir.path().join("nowhere");
    let failed = as_named(&nowhere, "Ada");
    assert!(
        matches!(&failed, Err(GitError::Failed { stderr, .. }) if stderr.contains("cannot change")),
        "{failed:?}"
    );
}

/// A secret git prints where a cap would cut it is redacted while whole,
/// so no part of it is kept: at the standard error cap, and at the
/// quoted message's.
#[test]
fn a_secret_straddling_a_cap_is_redacted_whole() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123";
    let at_cap = format!("{} {secret} tail", "x".repeat(MAX_STDERR - 10));
    let kept = String::from_utf8(redacted_stderr(at_cap.as_bytes(), false)).unwrap();
    assert!(kept.len() <= MAX_STDERR);
    assert!(!kept.contains("ghp_"), "{}", &kept[kept.len() - 20..]);
    // The read itself cut the secret (git printed more): its raw tail goes
    // before redaction, so no prefix of it survives.
    let cut = format!("{} {}", "z".repeat(100), &secret[..12]);
    let tail = String::from_utf8(redacted_stderr(cut.as_bytes(), true)).unwrap();
    assert!(!tail.contains("ghp_"), "{tail}");
    assert_eq!(tail, "z".repeat(100));
    let at_quote = format!("{} {secret} tail", "y".repeat(MAX_QUOTED_CHARS - 10));
    let quote = quoted(at_quote.as_bytes());
    assert!(!quote.contains("ghp_"), "{}", &quote[quote.len() - 20..]);
    assert!(quoted(format!("https://u:{secret}@host/x").as_bytes()).contains("***@host"));
    // An escape sequence splitting a secret is dropped before redaction.
    let split = format!("{}\u{1b}[31m{}", &secret[..10], &secret[10..]);
    assert!(
        !quoted(split.as_bytes()).contains("ghp_"),
        "{}",
        quoted(split.as_bytes())
    );
    // A name is shown as it is: sanitised, never redacted.
    assert_eq!(shown_name(b"sk-abcdefgh.json"), "sk-abcdefgh.json");
    assert_eq!(shown_name(b"token=x.json"), "token=x.json");
}

/// Set for the test binary run as the parent that is killed: the
/// repository and the `PATH` its git runs with.
#[cfg(target_os = "linux")]
const KILLED_PARENT: [&str; 2] = [
    "QUECTO_TEST_KILLED_PARENT_REPO",
    "QUECTO_TEST_KILLED_PARENT_PATH",
];

/// As the parent the next tests kill: git through the runner, hanging.
#[cfg(target_os = "linux")]
#[test]
fn a_parent_that_runs_a_hanging_git() {
    let (Some(repo), Some(path)) = (
        std::env::var_os(KILLED_PARENT[0]),
        std::env::var_os(KILLED_PARENT[1]),
    ) else {
        return;
    };
    let env = GitEnvironment::allowlisted([("PATH".into(), path)]);
    let git = Git::new(Path::new(&repo), env, person("Ada")).unwrap();
    let _ = git.network(&["fetch"]);
}

/// A quecto running `script` as its git, killed outright: whether each
/// process named in `pid_files` was gone, once that is `expected` or
/// within five seconds.
#[cfg(target_os = "linux")]
fn killed_with_git(
    repos: &Repos,
    script: &str,
    pid_files: &[&Path],
    expected: &[bool],
) -> Vec<bool> {
    use crate::infrastructure::processes::parent_death_signal::tests::KilledParent;
    drop(shim(repos, script, &[]));
    let mut path = repos.dir.path().join("bin").into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let name = "infrastructure::persistence::project_board::store::tests::runner::a_parent_that_runs_a_hanging_git";
    let vars = [
        (KILLED_PARENT[0], repos.alice.as_os_str()),
        (KILLED_PARENT[1], path.as_os_str()),
    ];
    KilledParent::start(name, &vars, pid_files).kill(std::time::Duration::from_secs(5), expected)
}

/// When quecto is killed outright (a relay's timeout, say), the git it
/// was running is asked to stop with SIGTERM, so it can clean up (a ref
/// lock, the children it waits on) as it goes, its own session
/// notwithstanding.
#[cfg(target_os = "linux")]
#[test]
fn a_git_is_terminated_when_the_quecto_running_it_is_killed() {
    let repos = Repos::new();
    let (pid, signal) = (
        repos.dir.path().join("pid"),
        repos.dir.path().join("signal"),
    );
    let script = format!(
        "#!/bin/sh\ntrap 'echo TERM > \"{}\"; kill $!; exit 0' TERM\necho $$ > '{}'\nsleep 60 &\nwait\n",
        signal.display(),
        pid.display()
    );
    assert_eq!(
        killed_with_git(&repos, &script, &[&pid], &[true]),
        [true],
        "git outlived quecto"
    );
    assert_eq!(
        std::fs::read_to_string(&signal).ok().as_deref(),
        Some("TERM\n")
    );
}

/// The limit, made explicit: the death signal reaches git alone. A process
/// git starts and leaves running (an ssh, a credential helper) is not
/// signalled; real git ends the children it waits on when it is
/// terminated, which this stub does not.
#[cfg(target_os = "linux")]
#[test]
fn a_grandchild_git_leaves_running_outlives_the_killed_quecto() {
    let repos = Repos::new();
    let (pid, grandchild) = (
        repos.dir.path().join("pid"),
        repos.dir.path().join("grandchild"),
    );
    let script = format!(
        "#!/bin/sh\nsh -c 'echo $$ > \"{}\"; exec sleep 60' &\necho $$ > '{}'\nwait\n",
        grandchild.display(),
        pid.display()
    );
    assert_eq!(
        killed_with_git(&repos, &script, &[&pid, &grandchild], &[true, false]),
        [true, false]
    );
}

#[path = "git_command_config_tests.rs"]
mod command_config;
