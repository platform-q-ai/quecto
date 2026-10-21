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
    })
    .unwrap_err();
    assert!(error.contains("cannot start"), "{error}");
    let error = spawn_detached(&DetachedCommand {
        program: PathBuf::from("/bin/true"),
        args: Vec::new(),
        env: Vec::new(),
        log: temp.path().join("no/such/dir/detached.log"),
        tether: Tether::Detached,
    })
    .unwrap_err();
    assert!(error.contains("detached.log"), "{error}");
}
