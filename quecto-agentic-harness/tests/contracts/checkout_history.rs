//! `CheckoutHistory` on the git adapter (#2687): every commit asked about
//! is an ancestor of the descendant (itself included), or the answer says
//! why not: not an ancestor, not a full commit id, git failing (an unknown
//! commit, no repository), or the one deadline passing, its git ended and
//! reaped. The adapter's git ignores this process's `GIT_DIR` and
//! `GIT_WORK_TREE`, and inherits command-scope configuration
//! (`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`).
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use quecto::application::swarm::ports::board_files::{Ancestry, CheckoutHistory};
use quecto::infrastructure::workspace::checkout_history::GitCheckoutHistory;

/// git in `dir` with no user or system configuration and no inherited git
/// environment: its trimmed output.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=t@example.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        // Only PATH passes: a caller's GIT_DIR or GIT_INDEX_FILE,
        // including another contract's, must never reach this setup.
        // These compiled tests run in CI, not the fast local pre-push gate.
        .env_clear()
        .envs(std::env::var_os("PATH").map(|path| ("PATH", path)))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout)
        .expect("git prints text")
        .trim_end()
        .to_owned()
}

/// A repository with two commits, `base` and `next` on it, and a commit
/// outside that history.
struct Repository {
    dir: tempfile::TempDir,
    base: String,
    next: String,
    unrelated: String,
}

fn repository() -> Repository {
    repository_named("base")
}

/// [`repository`] whose base commit says `base`: repositories built in the
/// same second share their commit ids unless their messages differ.
fn repository_named(base: &str) -> Repository {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", base]);
    let base = git(root, &["rev-parse", "HEAD"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "next"]);
    let next = git(root, &["rev-parse", "HEAD"]);
    let tree = git(root, &["rev-parse", "HEAD^{tree}"]);
    let unrelated = git(root, &["commit-tree", &tree, "-m", "unrelated"]);
    Repository {
        dir,
        base,
        next,
        unrelated,
    }
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn only_confirmed_ancestors_are_confirmed() {
    let repo = repository();
    let (base, next, unrelated) = (&repo.base, &repo.next, &repo.unrelated);
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    let history = GitCheckoutHistory::new(repo.dir.path());
    for (ancestors, descendant, answer) in [
        (ids(&[base]), next.as_str(), Ancestry::Confirmed),
        (ids(&[base, next]), next.as_str(), Ancestry::Confirmed),
        (ids(&[next]), next.as_str(), Ancestry::Confirmed),
        (ids(&[next]), base.as_str(), Ancestry::NotAncestor),
        (ids(&[base]), unrelated.as_str(), Ancestry::NotAncestor),
        (
            ids(&[base, unrelated]),
            next.as_str(),
            Ancestry::NotAncestor,
        ),
        (ids(&[unknown]), next.as_str(), Ancestry::GitFailed),
        (ids(&[base]), unknown, Ancestry::GitFailed),
        (ids(&[&base[..12]]), next.as_str(), Ancestry::NotFullId),
        (ids(&["HEAD~1"]), "HEAD", Ancestry::NotFullId),
        (ids(&["--all"]), next.as_str(), Ancestry::NotFullId),
        (ids(&[]), next.as_str(), Ancestry::NotFullId),
    ] {
        assert_eq!(
            history.ancestry(&ancestors, descendant),
            answer,
            "{ancestors:?} {descendant}"
        );
    }
    let elsewhere = tempfile::tempdir().unwrap();
    assert_eq!(
        GitCheckoutHistory::new(elsewhere.path()).ancestry(&ids(&[base]), next),
        Ancestry::GitFailed
    );
}

/// #2691 review L2: a `GIT_DIR` and `GIT_WORK_TREE` exported in this
/// process (a hook's, an IDE's) name another repository; the adapter's git
/// still answers about the checkout it is bound to. Deleting the adapter's
/// `env_clear()` fails this test.
#[test]
fn an_ambient_git_dir_does_not_redirect_the_question() {
    let repo = repository();
    let other = repository_named("another base");
    assert_ne!(repo.base, other.base, "the repositories differ");
    let ancestors = ids(&[&repo.base]);
    let _env = crate::GIT_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Restored below, before the lock is released.
    // SAFETY: GIT_ENV_LOCK serialises this process-wide mutation.
    unsafe {
        std::env::set_var("GIT_DIR", other.dir.path().join(".git"));
        std::env::set_var("GIT_WORK_TREE", other.dir.path());
    }
    let history = GitCheckoutHistory::new(repo.dir.path());
    let ambient = history.ancestry(&ancestors, &repo.next);
    // SAFETY: GIT_ENV_LOCK is still held; this restores the environment.
    unsafe {
        std::env::remove_var("GIT_DIR");
        std::env::remove_var("GIT_WORK_TREE");
    }
    assert_eq!(ambient, Ancestry::Confirmed);
}

/// `dir` first, then this process's own `PATH` (for the fake's `sleep`).
fn with_system_path(dir: &Path) -> OsString {
    let mut path = dir.as_os_str().to_owned();
    if let Some(system) = std::env::var_os("PATH") {
        path.push(":");
        path.push(system);
    }
    path
}

/// A directory holding an executable `git` that runs `script`.
fn fake_git(script: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("git");
    std::fs::write(&program, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

const BASE: &str = "db03de82dd7a7222b51a301bb9e8f0a01bada0f3";
const COMMIT: &str = "eb4fe3fd96771037e7d9162013d22036fa42d82a";

/// #2691 review L3: a git that does not answer is ended at the one
/// deadline, the answer is `TimedOut` well before git would have finished,
/// and the child is reaped (no zombie is left under its pid).
#[test]
fn git_past_the_deadline_is_ended_and_reaped() {
    let pid_file = tempfile::tempdir().unwrap();
    let pid_path = pid_file.path().join("pid");
    let fake = fake_git(&format!(
        "echo $$ > '{}'\nexec sleep 30",
        pid_path.display()
    ));
    let checkout = tempfile::tempdir().unwrap();
    let history = GitCheckoutHistory::with_environment(
        checkout.path(),
        [(OsString::from("PATH"), with_system_path(fake.path()))],
        Duration::from_millis(300),
    );
    let started = Instant::now();
    let answer = history.ancestry(&ids(&[BASE, BASE]), COMMIT);
    let took = started.elapsed();
    assert_eq!(answer, Ancestry::TimedOut);
    assert!(took >= Duration::from_millis(300), "{took:?}");
    assert!(
        took < Duration::from_secs(5),
        "ended at the deadline, not when git would finish: {took:?}"
    );
    let pid = std::fs::read_to_string(&pid_path).expect("the fake git ran");
    let pid = pid.trim();
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "git {pid} was not reaped"
    );
}

/// #2691 review L4: command-scope configuration reaches git (a
/// container's `safe.directory` set through `GIT_CONFIG_COUNT`), and a
/// repository redirect does not.
#[test]
fn command_scope_configuration_reaches_git() {
    let fake = fake_git(
        "[ \"$GIT_CONFIG_COUNT\" = 1 ] && [ \"$GIT_CONFIG_KEY_0\" = safe.directory ] \
         && [ \"$GIT_CONFIG_VALUE_0\" = '*' ] && [ -z \"$GIT_DIR\" ] && exit 0\nexit 1",
    );
    let checkout = tempfile::tempdir().unwrap();
    let environment = |extra: &[(&str, &str)]| {
        let mut environment = vec![(OsString::from("PATH"), fake.path().as_os_str().to_owned())];
        environment.extend(
            extra
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value))),
        );
        environment
    };
    let configured = [
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "safe.directory"),
        ("GIT_CONFIG_VALUE_0", "*"),
    ];
    let history = GitCheckoutHistory::with_environment(
        checkout.path(),
        environment(&configured),
        Duration::from_secs(10),
    );
    assert_eq!(history.ancestry(&ids(&[BASE]), COMMIT), Ancestry::Confirmed);
    let redirected = [configured.as_slice(), &[("GIT_DIR", "/elsewhere/.git")]].concat();
    let history = GitCheckoutHistory::with_environment(
        checkout.path(),
        environment(&redirected),
        Duration::from_secs(10),
    );
    assert_eq!(history.ancestry(&ids(&[BASE]), COMMIT), Ancestry::Confirmed);
    let history = GitCheckoutHistory::with_environment(
        checkout.path(),
        environment(&[]),
        Duration::from_secs(10),
    );
    assert_eq!(
        history.ancestry(&ids(&[BASE]), COMMIT),
        Ancestry::NotAncestor
    );
}

/// #2691 round-2 L2: the deadline is one for the whole question, not one
/// per commit. Each call of this git answers "yes" after 200 ms; with two
/// commits to ask about and a 300 ms deadline, the second call is ended at
/// the deadline. A deadline per commit would let both finish.
#[test]
fn the_deadline_covers_every_commit_asked_about() {
    let fake = fake_git("sleep 0.2\nexit 0");
    let checkout = tempfile::tempdir().unwrap();
    let history = GitCheckoutHistory::with_environment(
        checkout.path(),
        [(OsString::from("PATH"), with_system_path(fake.path()))],
        Duration::from_millis(300),
    );
    let started = Instant::now();
    assert_eq!(
        history.ancestry(&ids(&[BASE, BASE]), COMMIT),
        Ancestry::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    // One commit fits within the deadline.
    assert_eq!(history.ancestry(&ids(&[BASE]), COMMIT), Ancestry::Confirmed);
}

/// #2691 round-2 L3: a declined carry is traced with its reason.
#[test]
fn a_declined_carry_is_traced_with_its_reason() {
    let fake = fake_git("exit 1");
    let checkout = tempfile::tempdir().unwrap();
    let history = GitCheckoutHistory::with_environment(
        checkout.path(),
        [(OsString::from("PATH"), with_system_path(fake.path()))],
        Duration::from_secs(10),
    );
    let mut answer = None;
    let traced =
        quecto::infrastructure::test_support::tracing_capture::traced(tracing::Level::INFO, || {
            answer = Some(history.ancestry(&ids(&[BASE]), COMMIT))
        });
    assert_eq!(answer, Some(Ancestry::NotAncestor));
    assert!(
        traced.contains("swarm evidence carry declined")
            && traced.contains("reason=\"not_ancestor\""),
        "{traced}"
    );
    let confirmed = fake_git("exit 0");
    let history = GitCheckoutHistory::with_environment(
        checkout.path(),
        [(OsString::from("PATH"), with_system_path(confirmed.path()))],
        Duration::from_secs(10),
    );
    let traced =
        quecto::infrastructure::test_support::tracing_capture::traced(tracing::Level::INFO, || {
            assert_eq!(history.ancestry(&ids(&[BASE]), COMMIT), Ancestry::Confirmed)
        });
    assert!(!traced.contains("carry declined"), "{traced}");
}
