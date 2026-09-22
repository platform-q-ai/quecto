//! Termination signals and parent death on the CLI path (#2053).
use super::cli_cov_tests::{args, spawn_agent_program_retry_etxtbsy, tmp_dir};
use super::*;

/// [`spawn_agent_program_until`] with no signal to interrupt it.
pub(super) async fn spawn_agent_program(
    program: &str,
    flags: &CliFlags,
) -> Result<
    (
        PathBuf,
        tokio::process::Child,
        crate::shell::child_watch::StderrTail,
        Option<u8>,
    ),
    String,
> {
    let (_never, mut interrupt) = tokio::sync::mpsc::channel(1);
    spawn_agent_program_until(program, flags, &mut interrupt)
        .await
        .map_err(|abort| match abort {
            SpawnAbort::Failed(why) => why,
            SpawnAbort::Interrupted(signal) => format!("{} during startup", signal.name()),
        })
}

use std::path::PathBuf;
// ── termination signals and parent death (#2053) ───────────────────────

/// The startup check answers a signal that arrived before the loop, once.
#[tokio::test]
async fn a_signal_during_startup_is_taken_once_and_named() {
    use crate::shell::signals::TerminationSignal;
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    assert_eq!(interrupted_during_startup(&mut rx), None);
    tx.send(TerminationSignal::Hangup).await.unwrap();
    assert_eq!(
        interrupted_during_startup(&mut rx),
        Some(TerminationSignal::Hangup)
    );
    assert_eq!(interrupted_during_startup(&mut rx), None);
    assert_eq!(
        startup_interrupted_message(TerminationSignal::Terminate, true),
        "SIGTERM during startup: the agent being started is being terminated, nothing to persist"
    );
    assert_eq!(
        startup_interrupted_message(TerminationSignal::Terminate, false),
        "SIGTERM during startup: the agent being started is left running (--detach-on-exit), nothing to persist"
    );
    assert_eq!(
        termination_exit_message(TerminationSignal::Hangup, OwnedAgentAtExit::AskedToEnd),
        "SIGHUP: the session was asked to persist, the owned agent was asked to end"
    );
    assert_eq!(
        termination_exit_message(TerminationSignal::Interrupt, OwnedAgentAtExit::LeftRunning),
        "SIGINT: the session was asked to persist, the owned agent was left running (--detach-on-exit)"
    );
    assert_eq!(
        termination_exit_message(TerminationSignal::Terminate, OwnedAgentAtExit::NoneOwned),
        "SIGTERM: the session was asked to persist, no agent is owned (attached)"
    );
}

/// A stand-in that announces its socket only after `delay` seconds and
/// exits 0 on SIGTERM.
fn slow_agent(tag: &str, delay_secs: u32) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmp_dir(tag);
    let sock = dir.join("agent.sock");
    let script = dir.join("slow-agent.py");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env python3\n\
             import signal, sys, time\n\
             signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))\n\
             time.sleep({delay_secs})\n\
             print('quecto-agent-socket: {}', file=sys.stderr, flush=True)\n\
             signal.pause()\n",
            sock.display()
        ),
    )
    .expect("write slow agent");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, script)
}

#[cfg(target_os = "linux")]
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| s.get(s.rfind(')')? + 2..)?.chars().next())
        .is_some_and(|st| st != 'Z' && st != 'X')
}

/// A signal while the announcement is still pending interrupts the spawn
/// at once — not after the announcement, not after the deadline — and ends
/// the agent being started.
#[tokio::test]
async fn a_signal_while_waiting_for_the_announcement_interrupts_the_spawn_at_once() {
    use crate::shell::signals::TerminationSignal;
    let (_dir, script) = slow_agent("slow-owned", 30);
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let flags = parse_flags(&args(""));
    let started = std::time::Instant::now();
    let spawn = spawn_agent_program_until(script.to_str().unwrap(), &flags, &mut rx);
    let (aborted, _) = tokio::join!(spawn, async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        tx.send(TerminationSignal::Hangup).await.unwrap();
    });
    let abort = aborted.expect_err("interrupted");
    assert!(
        matches!(abort, SpawnAbort::Interrupted(TerminationSignal::Hangup)),
        "{abort:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "answered at once, not after the 30 s announcement"
    );
}

/// The same signal with --detach-on-exit waits for the announcement (so the
/// agent's announcing write never meets a closed pipe), then leaves the
/// agent running.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_signal_while_waiting_leaves_a_detached_agent_running_once_announced() {
    use crate::shell::signals::TerminationSignal;
    let (dir, script) = slow_agent("slow-detached", 1);
    let _listener = std::os::unix::net::UnixListener::bind(dir.join("agent.sock")).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let flags = parse_flags(&args("--detach-on-exit"));
    tx.send(TerminationSignal::Terminate).await.unwrap();
    let started = std::time::Instant::now();
    let abort = spawn_agent_program_until(script.to_str().unwrap(), &flags, &mut rx)
        .await
        .expect_err("interrupted");
    assert!(matches!(abort, SpawnAbort::Interrupted(_)), "{abort:?}");
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(900),
        "left only after the announcement (1 s away)"
    );
    // The stand-in is still there: find it by its script path, end it.
    let mine: Vec<u32> = std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|c| String::from_utf8_lossy(&c).contains(dir.to_str().unwrap()))
        })
        .collect();
    assert!(!mine.is_empty(), "the detached agent kept running");
    for pid in mine {
        assert!(alive(pid));
        // SAFETY: a pid whose command line names this test's private dir.
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

/// A stand-in that reports its own parent-death signal before announcing.
fn pdeathsig_reporting_agent(tag: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmp_dir(tag);
    let sock = dir.join("agent.sock");
    let script = dir.join("fake-agent.py");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env python3\n\
             import ctypes, signal, sys\n\
             v = ctypes.c_int()\n\
             ctypes.CDLL(None).prctl(2, ctypes.byref(v))\n\
             print('pdeathsig=%d' % v.value, file=sys.stderr, flush=True)\n\
             print('quecto-agent-socket: {}', file=sys.stderr, flush=True)\n\
             signal.pause()\n",
            sock.display()
        ),
    )
    .expect("write fake agent");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, script)
}

async fn spawned_pdeathsig(flag: &str, tag: &str) -> String {
    let (dir, script) = pdeathsig_reporting_agent(tag);
    let _listener = std::os::unix::net::UnixListener::bind(dir.join("agent.sock")).unwrap();
    let flags = parse_flags(&args(flag));
    let (_path, mut child, tail, _protocol) =
        spawn_agent_program_retry_etxtbsy(script.to_str().unwrap(), &flags)
            .await
            .expect("spawn fake agent");
    let reported = tail
        .lines()
        .iter()
        .find(|l| l.starts_with("pdeathsig="))
        .cloned()
        .expect("the stand-in reported its parent-death signal");
    let _ = child.kill().await;
    reported
}

/// The owned harness is armed with SIGTERM on parent death (Linux), so a TUI
/// that dies outright cannot orphan it; a detached one is not armed.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn the_spawned_harness_is_armed_for_parent_death_unless_detached() {
    assert_eq!(
        spawned_pdeathsig("", "pdeath-owned").await,
        format!("pdeathsig={}", libc::SIGTERM)
    );
    assert_eq!(
        spawned_pdeathsig("--detach-on-exit", "pdeath-detached").await,
        "pdeathsig=0"
    );
}
