//! The on-demand launcher over fake `quecto` scripts that exit at once
//! (#2615): which program it starts, with which arguments, in which
//! directory, and how it reads the way that broker ended.
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use std::ffi::OsString;

use super::*;
use crate::infrastructure::processes::local::detached_process::Scope;
use crate::infrastructure::test_support::executable::write_executable;

const LIMIT: Duration = Duration::from_secs(10);

/// A fake `quecto` in its own directory: it reports its argv and base
/// directory, then exits with `status`.
fn fake_quecto(dir: &Path, status: u8) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("quecto");
    write_executable(
        &script,
        format!(
            "#!/bin/sh\necho \"args: $*\"\necho \"base: $QUECTO_BASE_DIR\"\nenv | sed 's/=.*//' | sed 's/^/env: /'\nexit {status}\n"
        ),
    );
    script
}

fn launch_for(dir: &Path) -> AuthorityLaunch {
    AuthorityLaunch {
        directory: dir.join("admission"),
        config: dir.join("config.json"),
        base_dir: dir.to_path_buf(),
    }
}

fn settled(broker: &mut Box<dyn LaunchedAuthority>) -> LaunchState {
    let deadline = Instant::now() + LIMIT;
    loop {
        match broker.state() {
            LaunchState::Running => {
                assert!(Instant::now() < deadline, "the fake broker never exited");
                std::thread::sleep(Duration::from_millis(10));
            }
            settled => return settled,
        }
    }
}

fn log(dir: &Path) -> String {
    std::fs::read_to_string(DetachedAuthorityLauncher::log_path(&dir.join("admission")))
        .unwrap_or_default()
}

#[test]
fn the_probe_answers_only_when_a_broker_accepts() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("admission");
    let launcher = DetachedAuthorityLauncher::new(temp.path().join("quecto"));
    let error = launcher.probe(&directory).unwrap_err();
    assert!(error.contains("admission.sock"), "{error}");
    std::fs::create_dir_all(directory.join("client")).unwrap();
    let _listener =
        std::os::unix::net::UnixListener::bind(directory.join("client/admission.sock")).unwrap();
    assert_eq!(launcher.probe(&directory), Ok(()));
}

#[test]
fn only_the_quecto_binary_is_started() {
    let temp = tempfile::tempdir().unwrap();
    let launcher = DetachedAuthorityLauncher::new(PathBuf::from("/bin/sh"));
    let error = launcher.launch(&launch_for(temp.path())).err().unwrap();
    assert!(error.contains("is not the quecto binary"), "{error}");
    assert!(
        !temp.path().join("admission").exists(),
        "a refused launch creates nothing"
    );
    let unresolved = DetachedAuthorityLauncher {
        program: Err("cannot resolve the quecto binary: gone".into()),
        tether: Tether::Detached,
        scope: Scope::None,
    };
    let error = unresolved.launch(&launch_for(temp.path())).err().unwrap();
    assert!(
        error.contains("cannot resolve the quecto binary"),
        "{error}"
    );
}

#[test]
fn a_broker_is_started_to_serve_the_launch_and_a_busy_exit_lost_the_race() {
    let temp = tempfile::tempdir().unwrap();
    let launcher = DetachedAuthorityLauncher::new(fake_quecto(temp.path(), 3)).without_scope();
    let mut broker = launcher.launch(&launch_for(temp.path())).unwrap();
    assert_eq!(
        settled(&mut broker),
        LaunchState::LostRace,
        "{}",
        log(temp.path())
    );
    let log = log(temp.path());
    let directory = temp.path().join("admission");
    assert!(
        log.contains(&format!(
            "args: admission-broker run --stop-when-idle --config {} --directory {}\n",
            temp.path().join("config.json").display(),
            directory.display()
        )),
        "{log}"
    );
    assert!(
        log.contains(&format!("base: {}\n", temp.path().display())),
        "{log}"
    );
    // The directory is the broker's private one.
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&directory), 0o700);
    assert_eq!(mode(&directory.join("client")), 0o700);
}

#[test]
fn a_broker_that_exits_otherwise_failed_and_names_its_log() {
    let temp = tempfile::tempdir().unwrap();
    let launcher = DetachedAuthorityLauncher::new(fake_quecto(temp.path(), 1)).without_scope();
    let mut broker = launcher.launch(&launch_for(temp.path())).unwrap();
    let LaunchState::Failed { reason } = settled(&mut broker) else {
        panic!("an exit other than Busy is a failure");
    };
    let log_path = DetachedAuthorityLauncher::log_path(&temp.path().join("admission"));
    assert!(reason.contains(&log_path.display().to_string()), "{reason}");
    assert!(reason.contains("before it answered"), "{reason}");
    assert_eq!(broker.state(), LaunchState::Failed { reason });
}

#[test]
fn a_shared_directory_is_refused_before_anything_runs() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("admission");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    let launcher = DetachedAuthorityLauncher::new(fake_quecto(temp.path(), 3)).without_scope();
    let error = launcher.launch(&launch_for(temp.path())).err().unwrap();
    assert!(error.contains("0700"), "{error}");
    assert!(log(temp.path()).is_empty(), "nothing ran");
}

#[test]
fn pauses_follow_the_bounded_reconnect_backoff() {
    let launcher = DetachedAuthorityLauncher::new(PathBuf::from("/nonexistent/quecto"));
    let started = Instant::now();
    launcher.pause(0);
    let first = started.elapsed();
    assert!(first >= Duration::from_millis(100), "{first:?}");
    assert!(first < Duration::from_millis(1_000), "{first:?}");
}

/// #2615 review: the broker gets an allowlisted environment: no provider
/// keys or tokens, and a fixed `RUST_LOG`.
#[test]
fn the_broker_environment_is_an_allowlist() {
    let parent = [
        ("PATH", "/usr/bin"),
        ("HOME", "/home/me"),
        ("LC_ALL", "C.UTF-8"),
        ("OPENAI_API_KEY", "fake-key"),
        ("GH_TOKEN", "fake-token"),
        ("RUST_LOG", "debug"),
        ("QUECTO_BASE_DIR", "/elsewhere"),
    ]
    .map(|(name, value)| (OsString::from(name), OsString::from(value)));
    let env = broker_environment(parent, Path::new("/home/me/.quecto"));
    let env: Vec<(String, String)> = env
        .into_iter()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let pair = |name: &str, value: &str| (name.to_string(), value.to_string());
    assert_eq!(
        env,
        [
            pair("PATH", "/usr/bin"),
            pair("HOME", "/home/me"),
            pair("LC_ALL", "C.UTF-8"),
            pair("QUECTO_BASE_DIR", "/home/me/.quecto"),
            pair("RUST_LOG", "warn"),
        ]
    );
}

/// The started broker sees that environment and nothing else of this
/// process's.
#[test]
fn a_started_broker_sees_only_the_allowlisted_environment() {
    let temp = tempfile::tempdir().unwrap();
    let launcher = DetachedAuthorityLauncher::new(fake_quecto(temp.path(), 3)).without_scope();
    let mut broker = launcher.launch(&launch_for(temp.path())).unwrap();
    assert_eq!(settled(&mut broker), LaunchState::LostRace);
    let log = log(temp.path());
    // The shell adds its own few; everything else came from the allowlist.
    let shell = ["PWD", "SHLVL", "_", "OLDPWD"];
    for name in log.lines().filter_map(|line| line.strip_prefix("env: ")) {
        assert!(
            INHERITED.contains(&name)
                || name.starts_with("LC_")
                || ["QUECTO_BASE_DIR", "RUST_LOG"].contains(&name)
                || shell.contains(&name),
            "{name} reached the broker: {log}"
        );
    }
    assert!(log.contains("env: RUST_LOG\n"), "{log}");
}
