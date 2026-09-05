use super::*;

fn spawn(script: &str) -> tokio::process::Child {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.args(["-c", script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Match production: the child is its own process-group leader so
        // group-targeted termination has a real group to signal.
        .process_group(0);
    cmd.spawn().expect("spawn test child")
}

#[tokio::test]
async fn records_signal_exit_with_name() {
    let watch = watch_child(spawn("kill -ABRT $$"), StderrTail::default());
    let detail = watch.wait_exit_detail(Duration::from_secs(5)).await;
    assert_eq!(
        detail.as_deref(),
        Some("agent process aborted: signal 6 (SIGABRT)")
    );
    assert_eq!(watch.exit_detail(), detail);
}

#[tokio::test]
async fn records_nonzero_exit_code() {
    let watch = watch_child(spawn("exit 3"), StderrTail::default());
    let detail = watch.wait_exit_detail(Duration::from_secs(5)).await;
    assert_eq!(detail.as_deref(), Some("agent process exited with code 3"));
}

/// Termination goes through the watcher while it still owns the un-reaped
/// child: the long-running child's group is terminated promptly.
#[tokio::test]
async fn terminate_kills_a_running_child_group() {
    let watch = watch_child(spawn("sleep 30"), StderrTail::default());
    tokio::time::timeout(Duration::from_secs(5), watch.terminate())
        .await
        .expect("terminate must complete well within the grace window");
}

/// An observed exit keeps the leader identity pinned until cleanup, even
/// with no surviving group members. No recycled group can be targeted.
#[tokio::test]
async fn terminate_after_observed_exit_with_empty_group_signals_nothing() {
    let watch = watch_child(spawn("exit 0"), StderrTail::default());
    assert!(
        watch
            .wait_exit_detail(Duration::from_secs(5))
            .await
            .is_some(),
        "child exit must be observed first"
    );
    tokio::time::timeout(Duration::from_secs(1), watch.terminate())
        .await
        .expect("terminate on an empty group must return promptly");
}

/// A group member that outlives the leader is still cleaned up; observing
/// leader exit does not release the process-group ownership pin.
#[tokio::test]
async fn terminate_after_observed_exit_kills_surviving_group_members() {
    // The leader backgrounds a long sleep into its group and exits.
    let pid_file = std::env::temp_dir().join(format!(
        "quecto-child-watch-reaped-survivor-{}",
        std::process::id()
    ));
    let script = format!(
        "python3 -c 'import os, time; open(\"{}\", \"w\").write(str(os.getpid())); time.sleep(30)' & exit 0",
        pid_file.display()
    );
    let child = spawn(&script);
    let watch = watch_child(child, StderrTail::default());
    let survivor = wait_for_pid_file(&pid_file).await;
    #[cfg(target_os = "linux")]
    let _cleanup = ExactTestCleanup::new(survivor);
    assert!(
        watch
            .wait_exit_detail(Duration::from_secs(5))
            .await
            .is_some(),
        "leader exit must be observed first"
    );
    tokio::time::timeout(Duration::from_secs(5), watch.terminate())
        .await
        .expect("terminate must complete within the grace window");
    assert_process_gone(survivor).await;
    let _ = std::fs::remove_file(pid_file);
}

/// The wait is event-driven: an exit already recorded resolves without
/// consuming the timeout; a child that never exits resolves `None` at it.
#[tokio::test]
async fn wait_times_out_to_none_while_child_lives() {
    let watch = watch_child(spawn("sleep 30"), StderrTail::default());
    let detail = watch.wait_exit_detail(Duration::from_millis(50)).await;
    assert_eq!(detail, None);
    watch.terminate().await;
}

/// #1047: the stderr ring buffer keeps only the newest lines — a chatty
/// child cannot grow it without bound, and the panic message (written
/// last) is always retained.
#[test]
fn stderr_tail_keeps_only_newest_lines() {
    let tail = StderrTail::default();
    for i in 0..(STDERR_TAIL_MAX_LINES + 5) {
        tail.push(format!("line {i}"));
    }
    let lines = tail.lines();
    assert_eq!(lines.len(), STDERR_TAIL_MAX_LINES);
    assert_eq!(lines.first().map(String::as_str), Some("line 5"));
    assert_eq!(
        lines.last().map(String::as_str),
        Some(&*format!("line {}", STDERR_TAIL_MAX_LINES + 4))
    );
}

/// #1051 final review: the drain-completion signal is event-driven —
/// times out while pending, resolves immediately once marked.
#[tokio::test]
async fn wait_drained_times_out_before_mark_and_resolves_after() {
    let tail = StderrTail::default();
    assert!(!tail.wait_drained(Duration::from_millis(20)).await);
    tail.mark_drained();
    assert!(tail.wait_drained(Duration::from_millis(20)).await);
}

/// #1047: the watch handle exposes the drained stderr tail so the
/// disconnect path can include it in the diagnostics.
#[tokio::test]
async fn watch_exposes_stderr_tail_lines() {
    let tail = StderrTail::default();
    tail.push("thread 'main' panicked at src/lib.rs:1: boom".to_string());
    let watch = watch_child(spawn("exit 0"), tail);
    assert_eq!(
        watch.stderr_tail_lines(),
        vec!["thread 'main' panicked at src/lib.rs:1: boom".to_string()]
    );
    watch.wait_exit_detail(Duration::from_secs(5)).await;
}

#[test]
fn unknown_signal_has_no_name_suffix() {
    use std::os::unix::process::ExitStatusExt;
    let status = std::process::ExitStatus::from_raw(34); // signal 34 (real-time)
    assert_eq!(describe_exit(status), "agent process aborted: signal 34");
}

#[tokio::test]
async fn terminate_waits_for_term_resistant_same_group_descendant_after_leader_exits() {
    let pid_file = std::env::temp_dir().join(format!(
        "quecto-child-watch-descendant-{}",
        std::process::id()
    ));
    let script = format!(
        "trap 'exit 0' TERM; python3 -c 'import os, signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); signal.signal(signal.SIGHUP, signal.SIG_IGN); signal.signal(signal.SIGINT, signal.SIG_IGN); open(\"{}\", \"w\").write(str(os.getpid())); time.sleep(30)' & while :; do sleep 1; done",
        pid_file.display()
    );
    let child = spawn(&script);
    let watch = watch_child(child, StderrTail::default());
    let descendant = wait_for_pid_file(&pid_file).await;
    #[cfg(target_os = "linux")]
    let _cleanup = ExactTestCleanup::new(descendant);

    tokio::time::timeout(Duration::from_secs(5), watch.terminate())
        .await
        .expect("terminate must not acknowledge until cleanup completes");

    assert_process_gone(descendant).await;
    let _ = std::fs::remove_file(pid_file);
}

async fn wait_for_pid_file(path: &std::path::Path) -> i32 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(contents) = std::fs::read_to_string(path) {
            if let Ok(pid) = contents.trim().parse() {
                return pid;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "descendant pid file not written"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn assert_process_gone(pid: i32) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        // SAFETY: pid was written by the test descendant; signal 0 only probes liveness.
        let probe = unsafe { libc::kill(pid, 0) };
        if probe == -1 || proc_state(pid).is_some_and(|state| state == 'Z' || state == 'X') {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "descendant process {pid} still alive"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn proc_state(pid: i32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    stat.get(close + 2..)?.chars().next()
}

/// The unrelated process is deliberately in its own group too. Cleanup must
/// follow ancestry, not all groups owned by this Unix user or session.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn owned_separate_group_is_killed_without_signalling_unrelated_group() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("owned.pid");
    let script = format!(
        "trap 'exit 0' TERM; python3 -c 'import os, signal, time; os.setsid(); signal.signal(signal.SIGTERM, signal.SIG_IGN); open(\"{}\", \"w\").write(str(os.getpid())); time.sleep(30)' & while :; do sleep 1; done",
        file.display()
    );
    let mut unrelated = spawn("exec sleep 30");
    let watch = watch_child(spawn(&script), StderrTail::default());
    let pid = wait_for_pid_file(&file).await;
    let cleanup = ExactTestCleanup::new(pid);
    let success = watch.terminate_with_timeout(Duration::from_secs(3)).await;
    let unrelated_survived = unrelated.try_wait().unwrap().is_none();
    unrelated.kill().await.unwrap();
    assert!(success, "ack requires verified descendant exit");
    assert!(
        unrelated_survived,
        "unrelated group must not receive cleanup signals"
    );
    assert_process_gone(pid).await;
    drop(cleanup);
}

#[cfg(target_os = "linux")]
struct ExactTestCleanup(std::os::fd::OwnedFd);
#[cfg(target_os = "linux")]
impl ExactTestCleanup {
    fn new(pid: i32) -> Self {
        use std::os::fd::FromRawFd;
        // SAFETY: test PID comes from the ready file; pin it before assertions.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        assert!(fd >= 0);
        // SAFETY: pidfd_open returned a new descriptor owned by this guard.
        Self(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) })
    }
}
#[cfg(target_os = "linux")]
impl Drop for ExactTestCleanup {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // SAFETY: guard owns the exact test-process handle, immune to PID reuse.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.0.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    }
}

#[tokio::test]
async fn dropping_watch_detaches_live_child_without_killing_it() {
    let mut child = spawn("exec sleep 30");
    let pid = child.id().unwrap();
    // Create a separate owned child for the actual watcher: leave this one as
    // an unrelated control that is explicitly reaped below.
    let watched = spawn("exec sleep 30");
    let watched_pid = watched.id().unwrap();
    let watch = watch_child(watched, StderrTail::default());
    drop(watch);
    tokio::time::sleep(Duration::from_millis(100)).await;
    // SAFETY: signal zero only observes the two test processes.
    let alive = unsafe { libc::kill(watched_pid as i32, 0) == 0 && libc::kill(pid as i32, 0) == 0 };
    // SAFETY: the detached test process is still alive here and was spawned by this test.
    unsafe {
        libc::kill(watched_pid as i32, libc::SIGKILL);
    }
    child.kill().await.unwrap();
    assert!(
        alive,
        "dropping a watch is detach, not ordinary killing exit"
    );
}
