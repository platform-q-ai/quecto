//! Termination signals and parent death on the CLI path (#2053).
use super::cli_cov_tests::{args, spawn_agent_program_retry_etxtbsy, tmp_dir};
use super::*;
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
        startup_interrupted_message(TerminationSignal::Terminate),
        "SIGTERM during startup: the agent being started is terminated, nothing to persist"
    );
    assert_eq!(
        termination_exit_message(TerminationSignal::Hangup, true),
        "SIGHUP: session persisted, the owned agent was asked to end"
    );
    assert_eq!(
        termination_exit_message(TerminationSignal::Interrupt, false),
        "SIGINT: session persisted, the owned agent was left running (--detach-on-exit)"
    );
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
