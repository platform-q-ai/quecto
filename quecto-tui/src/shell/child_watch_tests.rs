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

/// #1051 review (PID-reuse race): once the watcher has reaped the child
/// and its group is empty, terminate signals nothing — the liveness probe
/// fails, so a possibly recycled PGID is never targeted.
#[tokio::test]
async fn terminate_after_reap_with_empty_group_signals_nothing() {
    let watch = watch_child(spawn("exit 0"), StderrTail::default());
    assert!(
        watch
            .wait_exit_detail(Duration::from_secs(5))
            .await
            .is_some(),
        "child must be reaped first"
    );
    tokio::time::timeout(Duration::from_secs(1), watch.terminate())
        .await
        .expect("terminate on an empty group must return promptly");
}

/// #1051 final review (sub-agent orphan leak): a group member that
/// outlives the reaped leader — the sub-agent case — is still terminated
/// on TUI exit. While a member lives the PGID cannot be recycled, so the
/// probed group signal is safe.
#[tokio::test]
async fn terminate_after_reap_kills_surviving_group_members() {
    // The leader backgrounds a long sleep into its group and exits.
    let pid_file = std::env::temp_dir().join(format!(
        "quecto-tui-child-watch-survivor-{}-{}.pid",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    let _ = std::fs::remove_file(&pid_file);
    let child = spawn(&format!(
        "sleep 30 & echo $! > '{}' ; exit 0",
        pid_file.display()
    ));
    let pgid = child.id().expect("child pid") as i32;
    let watch = watch_child(child, StderrTail::default());
    assert!(
        watch
            .wait_exit_detail(Duration::from_secs(5))
            .await
            .is_some(),
        "leader must be reaped first"
    );
    let survivor_pid: i32 = std::fs::read_to_string(&pid_file)
        .expect("survivor pid recorded")
        .trim()
        .parse()
        .expect("survivor pid is numeric");
    tokio::time::timeout(Duration::from_secs(5), watch.terminate())
        .await
        .expect("terminate must complete within the grace window");
    // A SIGKILLed orphan can linger briefly as an init-owned zombie before
    // it is reaped. That is no longer an executing survivor, so treat a
    // zombie state as terminated instead of waiting for unrelated init timing.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if !process_is_running_non_zombie(survivor_pid) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the surviving group member must be gone"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // SAFETY: pgid > 0 (from child.id()), so -pgid targets that group; signal 0 only probes for members without delivering a signal.
    let group_probe = unsafe { libc::kill(-pgid, 0) };
    assert!(
        group_probe == -1 || process_is_zombie(survivor_pid),
        "the group may remain probeable only while init still owns a zombie survivor"
    );
    let _ = std::fs::remove_file(pid_file);
}

fn process_is_running_non_zombie(pid: i32) -> bool {
    // SAFETY: pid > 0 from the child-reported `$!`; signal 0 only probes for liveness.
    if unsafe { libc::kill(pid, 0) } == -1 {
        return false;
    }
    !process_is_zombie(pid)
}

#[cfg(target_os = "linux")]
fn process_is_zombie(pid: i32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, rest)| rest.chars().next())
        == Some('Z')
}

#[cfg(not(target_os = "linux"))]
fn process_is_zombie(_pid: i32) -> bool {
    false
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
