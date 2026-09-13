use super::*;

// --- checked_pid tests ---

#[test]
fn checked_pid_normal_value() {
    assert_eq!(checked_pid(1234), Ok(1234));
}

#[test]
fn checked_pid_one() {
    assert_eq!(checked_pid(1), Ok(1));
}

#[test]
fn checked_pid_i32_max() {
    let max = i32::MAX as u32; // 2_147_483_647
    assert_eq!(checked_pid(max), Ok(i32::MAX));
}

#[test]
fn checked_pid_i32_max_plus_one_is_err() {
    let overflow = (i32::MAX as u32) + 1; // 2_147_483_648
    assert_eq!(checked_pid(overflow), Err(QuectodError::Overflow(overflow)));
}

#[test]
fn checked_pid_u32_max_is_err() {
    assert_eq!(checked_pid(u32::MAX), Err(QuectodError::Overflow(u32::MAX)));
}

#[test]
fn checked_pid_u32_max_does_not_produce_negative_one() {
    // This is the critical safety check: u32::MAX as i32 == -1,
    // and kill(-(-1), sig) == kill(1, sig) == kill init.
    let result = checked_pid(u32::MAX);
    assert!(result.is_err());
    // Verify the old unchecked cast WOULD have produced -1:
    assert_eq!(u32::MAX as i32, -1, "confirms the wrapping cast danger");
}

#[test]
fn checked_pid_zero_is_err() {
    assert_eq!(checked_pid(0), Err(QuectodError::Zero));
}

#[test]
fn pid_error_display_overflow() {
    let e = QuectodError::Overflow(2_147_483_648);
    let msg = e.to_string();
    assert!(msg.contains("2147483648"), "should mention the PID value");
    assert!(msg.contains("i32::MAX"), "should mention the limit");
}

#[test]
fn pid_error_display_zero() {
    let e = QuectodError::Zero;
    let msg = e.to_string();
    assert!(msg.contains("PID 0"), "should mention PID 0");
}

#[test]
fn pid_error_implements_std_error() {
    let e: Box<dyn std::error::Error> = Box::new(QuectodError::Zero);
    assert!(e.to_string().contains("PID 0"));
}

// --- signal_leader: one pid, never a group ---

#[test]
fn signal_leader_refuses_zero_and_negative_pids() {
    // kill(0) / kill(-n) address a process group; the helper must refuse
    // them rather than ever widening into a group signal.
    assert_eq!(signal_leader(0, 0), -1);
    assert_eq!(signal_leader(-1, 0), -1);
    assert_eq!(signal_leader(i32::MIN, 0), -1);
}

#[test]
fn signal_leader_with_nonexistent_pid_returns_error() {
    // Signal 0 only probes; ESRCH for a pid that does not exist.
    assert_eq!(signal_leader(999_999_998, 0), -1);
}

#[test]
fn signal_leader_probe_of_self_succeeds() {
    let me = checked_pid(std::process::id()).unwrap();
    assert_eq!(signal_leader(me, 0), 0);
}

// --- /proc stat parsing ---

#[test]
fn parse_parent_and_group_reads_fields_after_the_comm() {
    let stat = "1234 (sleep (x) y) S 77 88 99 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 5 0 0";
    assert_eq!(parse_parent_and_group(stat), Some((77, 88)));
    assert_eq!(parse_parent_and_group("garbage"), None);
    assert_eq!(parse_parent_and_group("1 (a) S"), None);
}

#[test]
fn parse_start_time_reads_field_22() {
    let stat = "1234 (sleep (x) y) S 77 88 99 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 5 0 0";
    assert_eq!(parse_start_time(stat), Some(5));
    assert_eq!(parse_start_time("garbage"), None);
}

// --- terminate_leader ---

fn spawn_group(script: &str) -> tokio::process::Child {
    tokio::process::Command::new("sh")
        .args(["-c", script])
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn test child")
}

/// Short two-stage budget for fakes that ignore or delay TERM.
fn short(settle_ms: u64, force_ms: u64) -> LeaderBudget {
    LeaderBudget {
        settle: std::time::Duration::from_millis(settle_ms),
        force: std::time::Duration::from_millis(force_ms),
    }
}

async fn terminate(child: &mut tokio::process::Child, budget: LeaderBudget) -> LeaderTermination {
    let identity = LeaderIdentity::capture(child.id());
    terminate_leader(child, budget, identity).await
}

async fn wait_for_file(path: &std::path::Path) -> String {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && !text.trim().is_empty()
        {
            return text.trim().to_string();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{path:?} not written"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes liveness of a pid this test spawned.
    let probed = unsafe { libc::kill(pid, 0) == 0 };
    probed
        && std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| s.get(s.rfind(')')? + 2..)?.chars().next())
            .is_some_and(|state| state != 'Z' && state != 'X')
}

/// Poll until `pid` is gone (bounded): a `Killed` label without a dead
/// process would mean the SIGKILL was never sent.
async fn assert_gone(pid: i32) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while alive(pid) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "pid {pid} still alive after termination"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// A leader that settles its own child on SIGTERM, then exits: the child
/// is gone before the leader exits, the leader ends 0 after TERM, and the
/// TUI never signals the child (the child logs every signal it receives).
#[tokio::test]
async fn leader_settles_its_child_and_exits_without_the_tui_signalling_the_child() {
    let dir = tempfile::tempdir().unwrap();
    let child_pid = dir.path().join("child.pid");
    let child_log = dir.path().join("child.signals");
    let order = dir.path().join("order");
    let script = format!(
        "sh -c 'trap \"echo TERM >> {log}\" TERM; trap \"echo INT >> {log}\" INT; echo $$ > {pid}; while :; do sleep 0.05; done' & kid=$!; \
         trap 'kill -KILL $kid; wait $kid; echo child-gone >> {order}; exit 0' TERM; while :; do sleep 0.05; done",
        log = child_log.display(),
        pid = child_pid.display(),
        order = order.display(),
    );
    let mut leader = spawn_group(&script);
    let kid: i32 = wait_for_file(&child_pid).await.parse().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let outcome = terminate(&mut leader, short(10_000, 10_000)).await;

    assert_eq!(outcome.end, LeaderEnd::ExitedAfterTerm);
    assert_eq!(wait_for_file(&order).await, "child-gone");
    assert!(!alive(kid), "the leader settled its child before exiting");
    assert!(
        !child_log.exists(),
        "the TUI must never signal the child: {:?}",
        std::fs::read_to_string(&child_log)
    );
    assert!(outcome.strays.is_empty(), "{:?}", outcome.strays);
}

/// A leader that ignores SIGTERM twice is SIGKILLed — that one pid — only
/// after BOTH waits, is really dead afterwards, and the wait is bounded.
#[tokio::test]
async fn leader_ignoring_sigterm_is_killed_after_both_waits() {
    let mut leader = spawn_group("trap '' TERM; while :; do sleep 0.05; done");
    let pid = checked_pid(leader.id().unwrap()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let started = std::time::Instant::now();
    let budget = short(300, 200);

    let outcome = terminate(&mut leader, budget).await;

    let elapsed = started.elapsed();
    assert_eq!(outcome.end, LeaderEnd::Killed);
    assert!(
        elapsed >= budget.settle + budget.force,
        "KILL only after both waits: {elapsed:?}"
    );
    assert!(
        elapsed < budget.total(),
        "wait must end promptly once killed: {elapsed:?}"
    );
    assert_gone(pid).await;
}

/// A leader that ignores the first SIGTERM but exits on the repeated one
/// (the harness's force-exit shape) ends `ExitedAfterRepeatedTerm`, no KILL.
#[tokio::test]
async fn leader_exiting_on_the_repeated_sigterm_is_not_killed() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("second-term");
    let script = format!(
        "trap 'trap \": > {m}; exit 0\" TERM' TERM; while :; do sleep 0.05; done",
        m = marker.display()
    );
    let mut leader = spawn_group(&script);
    let pid = checked_pid(leader.id().unwrap()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let started = std::time::Instant::now();

    let outcome = terminate(&mut leader, short(300, 5_000)).await;

    assert_eq!(outcome.end, LeaderEnd::ExitedAfterRepeatedTerm);
    assert!(
        marker.exists(),
        "the second TERM's handler ran to completion"
    );
    assert!(started.elapsed() >= std::time::Duration::from_millis(300));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_gone(pid).await;
}

/// A slow-settling leader (3 s after TERM) exits inside the settle budget
/// with no second signal and no SIGKILL, and the wait ends as soon as it exits.
#[tokio::test]
async fn slow_settling_leader_exits_inside_the_budget_without_sigkill() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("finished");
    let script = format!(
        "trap 'trap \"exit 99\" TERM; sleep 3; : > {}; exit 0' TERM; while :; do sleep 0.05; done",
        marker.display()
    );
    let mut leader = spawn_group(&script);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let outcome = terminate(&mut leader, LeaderBudget::for_children(Some(0))).await;

    assert_eq!(outcome.end, LeaderEnd::ExitedAfterTerm);
    assert!(
        marker.exists(),
        "TERM handler ran to completion, no SIGKILL"
    );
    assert!(outcome.waited >= std::time::Duration::from_secs(3));
    assert!(outcome.waited < std::time::Duration::from_secs(10));
}

/// An already-exited leader is not signalled again; the canary still runs.
#[tokio::test]
async fn already_exited_leader_reports_without_signalling() {
    let mut leader = spawn_group("exit 0");
    let _ = leader.wait().await;
    let outcome = terminate(&mut leader, LeaderBudget::WORST_CASE).await;
    assert_eq!(
        outcome.end,
        LeaderEnd::NoPid,
        "reaped handle carries no pid"
    );
    let mut leader = spawn_group("exit 0");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let outcome = terminate(&mut leader, LeaderBudget::WORST_CASE).await;
    assert!(
        matches!(
            outcome.end,
            LeaderEnd::AlreadyExited | LeaderEnd::ExitedAfterTerm
        ),
        "{outcome:?}"
    );
}

/// The canary names a process that outlives the leader (same group) and
/// leaves it alone: it is still alive afterwards and received no signal.
#[tokio::test]
async fn canary_names_a_stray_without_signalling_it() {
    let dir = tempfile::tempdir().unwrap();
    let stray_pid = dir.path().join("stray.pid");
    let stray_log = dir.path().join("stray.signals");
    let script = format!(
        "sh -c 'trap \"echo TERM >> {log}\" TERM; echo $$ > {pid}; while :; do sleep 0.05; done' & \
         trap 'exit 0' TERM; while :; do sleep 0.05; done",
        log = stray_log.display(),
        pid = stray_pid.display(),
    );
    let mut leader = spawn_group(&script);
    let stray: i32 = wait_for_file(&stray_pid).await.parse().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let outcome = terminate(&mut leader, short(10_000, 10_000)).await;

    assert_eq!(outcome.end, LeaderEnd::ExitedAfterTerm);
    // The stray's own `sleep` grandchild shares the group, so it is named too.
    assert!(
        outcome.strays.contains(&stray),
        "canary names the stray: {:?}",
        outcome.strays
    );
    assert!(alive(stray), "the canary must not signal the stray");
    assert!(!stray_log.exists(), "stray received no signal");
    // SAFETY: the test owns the stray it spawned; clean it up.
    unsafe {
        libc::kill(stray, libc::SIGKILL);
    }
}

/// A recycled pid (a live process with the same pid but another start time)
/// disables the canary: its ppid/pgrp matches would belong to a stranger.
#[tokio::test]
async fn canary_is_skipped_when_the_leader_pid_was_recycled() {
    // Model recycling with a live process whose identity was captured with
    // a wrong start time: the pid exists, the start differs.
    let mut stranger =
        spawn_group("sh -c 'while :; do sleep 0.05; done' & while :; do sleep 0.05; done");
    let pid = stranger.id().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let genuine = LeaderIdentity::capture(Some(pid));
    assert!(genuine.start.is_some(), "start time captured from /proc");
    let stale = LeaderIdentity {
        pid: Some(pid),
        start: genuine.start.map(|s| s + 1),
    };
    assert!(
        !already_exited(genuine).strays.is_empty(),
        "the genuine identity sees the group's child"
    );
    assert!(
        already_exited(stale).strays.is_empty(),
        "a recycled pid runs no canary"
    );
    stranger.start_kill().unwrap();
    let _ = stranger.wait().await;
}

#[test]
fn canary_of_a_nonexistent_leader_is_empty() {
    assert!(stray_processes_of(999_999_997).is_empty());
    assert_eq!(
        already_exited(LeaderIdentity::capture(None)).end,
        LeaderEnd::NoPid
    );
}
