//! A detached start over fake scripts (#2615): one that exits at once is
//! started exactly as production starts the broker; one that keeps running
//! is tethered to the test thread, so it can never outlive the test (#2631).
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use super::*;
use crate::infrastructure::test_support::executable::write_executable;

const LIMIT: Duration = Duration::from_secs(10);

/// A script that reports its argv, working directory, environment, process
/// ids and whether it could read stdin, then runs `tail`.
fn script(dir: &Path, tail: &str) -> PathBuf {
    let path = dir.join("detached.sh");
    write_executable(
        &path,
        format!(
            "#!/bin/sh\n\
             echo \"args: $*\"\n\
             echo \"cwd: $(pwd)\"\n\
             echo \"env: $DETACHED_PROBE\"\n\
             echo \"home: $HOME\"\n\
             set -- $(cat /proc/$$/stat)\n\
             echo \"ids: pid=$1 pgrp=$5 sid=$6\"\n\
             read line && echo \"stdin: $line\"\n\
             echo oops >&2\n\
             {tail}\n"
        ),
    );
    path
}

fn command(dir: &Path, tail: &str, tether: Tether) -> DetachedCommand {
    DetachedCommand {
        program: script(dir, tail),
        args: vec!["one".into(), "two".into()],
        env: vec![("DETACHED_PROBE".into(), "seen".into())],
        log: dir.join("detached.log"),
        tether,
        scope: Scope::None,
        ran_marker: "args:".into(),
    }
}

fn exited(process: &mut DetachedProcess) -> Exit {
    let deadline = Instant::now() + LIMIT;
    loop {
        match process.exit() {
            None => {
                assert!(Instant::now() < deadline, "the script never exited");
                std::thread::sleep(Duration::from_millis(10));
            }
            Some(exit) => return exit,
        }
    }
}

fn log(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("detached.log")).unwrap_or_default()
}

#[test]
fn a_detached_process_leads_its_own_session_from_root_with_its_output_logged() {
    let temp = tempfile::tempdir().unwrap();
    let mut process = spawn_detached(&command(temp.path(), "exit 3", Tether::Detached)).unwrap();
    let exit = exited(&mut process).unwrap();
    assert_eq!(exit.code(), Some(3));
    // The outcome is kept: asking again answers the same.
    assert_eq!(process.exit(), Some(Ok(exit)));
    let log = log(temp.path());
    assert!(log.contains("args: one two\n"), "{log}");
    assert!(log.contains("cwd: /\n"), "{log}");
    assert!(log.contains("env: seen\n"), "{log}");
    assert!(
        log.contains("home: \n"),
        "nothing but the given environment: {log}"
    );
    assert!(log.contains("oops\n"), "stderr is logged too: {log}");
    assert!(!log.contains("stdin:"), "stdin reads nothing: {log}");
    let ids = log
        .lines()
        .find_map(|line| line.strip_prefix("ids: "))
        .expect("the script reported its ids");
    let ids: Vec<&str> = ids
        .split(' ')
        .map(|pair| pair.split_once('=').unwrap().1)
        .collect();
    assert_eq!(ids[0], ids[1], "it leads its process group: {log}");
    assert_eq!(ids[0], ids[2], "it leads its session: {log}");
    let mode = std::fs::metadata(temp.path().join("detached.log"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the log is owner-only");
}

#[test]
fn a_running_process_reads_running_and_its_log_is_appended_within_the_cap() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("detached.log"), "earlier start\n").unwrap();
    // Tethered: the test thread's end ends it, whatever happens here.
    let mut process = spawn_detached(&command(
        temp.path(),
        "exec sleep 30",
        Tether::DiesWithLaunchingThread,
    ))
    .unwrap();
    assert_eq!(process.exit(), None);
    // Dropping the handle hands it to a reaper; it keeps running.
    drop(process);
    let deadline = Instant::now() + LIMIT;
    while !log(temp.path()).contains("ids:") {
        assert!(Instant::now() < deadline, "the script never wrote its log");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(log(temp.path()).starts_with("earlier start\n"), "appended");
    // A log past its cap starts afresh.
    std::fs::write(
        temp.path().join("detached.log"),
        vec![b'x'; usize::try_from(LOG_CAP_BYTES).unwrap() + 1],
    )
    .unwrap();
    let mut quick = spawn_detached(&command(temp.path(), "exit 0", Tether::Detached)).unwrap();
    assert!(exited(&mut quick).unwrap().success());
    assert!(
        log(temp.path()).starts_with("args: "),
        "truncated past the cap"
    );
}

#[test]
fn a_program_that_cannot_start_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let error = spawn_detached(&DetachedCommand {
        program: temp.path().join("missing"),
        args: Vec::new(),
        env: Vec::new(),
        log: temp.path().join("detached.log"),
        tether: Tether::Detached,
        scope: Scope::None,
        ran_marker: "args:".into(),
    })
    .unwrap_err();
    assert!(error.contains("cannot start"), "{error}");
    let error = spawn_detached(&DetachedCommand {
        program: PathBuf::from("/bin/true"),
        args: Vec::new(),
        env: Vec::new(),
        log: temp.path().join("no/such/dir/detached.log"),
        tether: Tether::Detached,
        scope: Scope::None,
        ran_marker: "args:".into(),
    })
    .unwrap_err();
    assert!(error.contains("detached.log"), "{error}");
}

/// How a stand-in `systemd-run` behaves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Runner {
    /// Runs the program after `--` in place, as `systemd-run --scope` does.
    Execs,
    /// No user manager: every call fails.
    Refuses,
    /// The probe hangs past its bound.
    Slow,
    /// The probe passes, but starting the real program fails before exec.
    FailsToExec,
}

/// A stand-in `systemd-run` that records every call in `runner.log`.
fn runner(dir: &Path, behaviour: Runner) -> PathBuf {
    let path = dir.join("systemd-run");
    let before_exec = match behaviour {
        Runner::Execs => ":",
        Runner::Refuses => "exit 1",
        Runner::Slow => "[ \"$last\" = true ] && sleep 6",
        Runner::FailsToExec => {
            "[ \"$last\" = true ] || { echo 'Failed to start transient scope unit: Access denied' >&2; exit 1; }"
        }
    };
    write_executable(
        &path,
        format!(
            "#!/bin/sh\n\
             echo \"runner: $*\" >> '{marks}'\n\
             for last; do :; done\n\
             {before_exec}\n\
             while [ \"$1\" != \"--\" ]; do shift; done\n\
             shift\n\
             exec \"$@\"\n",
            marks = dir.join("runner.log").display(),
        ),
    );
    path
}

fn with_scope(dir: &Path, scope: Scope, tail: &str, tether: Tether) -> DetachedCommand {
    DetachedCommand {
        scope,
        // The runner and `true` are found on this PATH.
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        ..command(dir, tail, tether)
    }
}

fn marks(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("runner.log")).unwrap_or_default()
}

#[test]
fn a_reachable_user_manager_starts_it_in_a_transient_scope() {
    let temp = tempfile::tempdir().unwrap();
    let scope = Scope::SystemdRun(runner(temp.path(), Runner::Execs));
    let mut process =
        spawn_detached(&with_scope(temp.path(), scope, "exit 3", Tether::Detached)).unwrap();
    assert_eq!(process.started(), Started::InScope);
    assert_eq!(exited(&mut process).unwrap().code(), Some(3));
    let marks = marks(temp.path());
    assert!(
        marks.contains("runner: --user --scope --quiet --collect -- true\n"),
        "probed first: {marks}"
    );
    assert!(
        marks.contains(&format!(
            "runner: --user --scope --quiet --collect -- {} one two\n",
            temp.path().join("detached.sh").display()
        )),
        "then started through it: {marks}"
    );
    assert!(log(temp.path()).contains("args: one two\n"));
}

#[test]
fn without_a_user_manager_it_starts_plainly() {
    for (case, behaviour) in [("refused", Some(Runner::Refuses)), ("absent", None)] {
        let temp = tempfile::tempdir().unwrap();
        let runner = match behaviour {
            Some(behaviour) => runner(temp.path(), behaviour),
            None => temp.path().join("no-systemd-run"),
        };
        let mut process = spawn_detached(&with_scope(
            temp.path(),
            Scope::SystemdRun(runner),
            "exit 3",
            Tether::Detached,
        ))
        .unwrap();
        assert_eq!(process.started(), Started::Plain, "{case}");
        assert_eq!(exited(&mut process).unwrap().code(), Some(3), "{case}");
        let log = log(temp.path());
        assert!(log.contains("args: one two\n"), "{case}: {log}");
        assert_eq!(
            marks(temp.path()).lines().count(),
            usize::from(behaviour.is_some()),
            "{case}: only probed"
        );
    }
}

/// #2615 review round 2: a probe past its bound is no user manager: the
/// program starts plainly, and runs.
#[test]
fn a_probe_past_its_bound_starts_it_plainly() {
    let temp = tempfile::tempdir().unwrap();
    let scope = Scope::SystemdRun(runner(temp.path(), Runner::Slow));
    let started = Instant::now();
    let mut process = spawn_detached(&with_scope(
        temp.path(),
        scope,
        "exec sleep 30",
        Tether::DiesWithLaunchingThread,
    ))
    .unwrap();
    assert!(
        started.elapsed() >= SCOPE_PROBE_BOUND,
        "waited out the bound"
    );
    assert_eq!(process.started(), Started::Plain);
    assert_eq!(process.exit(), None, "the program runs");
    assert_eq!(marks(temp.path()).lines().count(), 1, "only probed");
}

/// #2615 review round 2: the runner answers its probe but fails to start
/// the program: the program is started plainly once.
#[test]
fn a_runner_that_fails_before_exec_falls_back_to_a_plain_start() {
    let temp = tempfile::tempdir().unwrap();
    let scope = Scope::SystemdRun(runner(temp.path(), Runner::FailsToExec));
    let mut process =
        spawn_detached(&with_scope(temp.path(), scope, "exit 3", Tether::Detached)).unwrap();
    assert_eq!(process.started(), Started::Plain);
    assert_eq!(exited(&mut process).unwrap().code(), Some(3));
    let log = log(temp.path());
    assert!(
        log.contains("Failed to start transient scope unit"),
        "{log}"
    );
    assert!(
        log.contains("args: one two\n"),
        "then started plainly: {log}"
    );
    assert_eq!(marks(temp.path()).lines().count(), 2, "probe, failed start");
}

/// A program that runs and fails through the runner is not retried.
#[test]
fn a_program_that_ran_and_failed_is_not_restarted_plainly() {
    let temp = tempfile::tempdir().unwrap();
    let scope = Scope::SystemdRun(runner(temp.path(), Runner::Execs));
    let mut process =
        spawn_detached(&with_scope(temp.path(), scope, "exit 1", Tether::Detached)).unwrap();
    assert_eq!(process.started(), Started::InScope);
    assert_eq!(exited(&mut process).unwrap().code(), Some(1));
    assert_eq!(
        log(temp.path()).matches("args: one two").count(),
        1,
        "ran once"
    );
}

/// A runner that cannot be started is named, not the program.
#[test]
fn a_runner_that_cannot_start_is_named() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("gone-runner");
    let mut command = command(temp.path(), "exit 3", Tether::Detached);
    command.program = temp.path().join("detached.sh");
    let error = start(&command, Some(&missing)).err().unwrap();
    assert!(error.contains("gone-runner"), "{error}");
    assert!(!error.contains("detached.sh"), "{error}");
}

/// #2615 review: a log is kept within its cap while its writer runs, and a
/// writer appending to it carries on at its new end.
#[test]
fn a_running_log_past_its_cap_is_emptied_and_appended_from_its_start() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("detached.log");
    let mut writer = open_log(&path).unwrap();
    writer.write_all(&[b'x'; 64]).unwrap();
    let reader = std::fs::File::open(&path).unwrap();
    assert!(!cap_log(&reader, 64).unwrap(), "at the cap: kept");
    writer.write_all(b"y").unwrap();
    assert!(cap_log(&writer, 64).unwrap(), "past it: emptied");
    writer.write_all(b"after\n").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\n");
    // A terminal or pipe is never touched.
    let (pipe, _keep) = std::io::pipe().unwrap();
    let pipe = File::from(std::os::fd::OwnedFd::from(pipe));
    assert!(!cap_log(&pipe, 0).unwrap());
}
