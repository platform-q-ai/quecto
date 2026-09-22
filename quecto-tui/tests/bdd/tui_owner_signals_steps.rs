//! #2053 steps for `tui_owner_signals.feature`: a REAL `quecto-tui` process
//! (the built binary) owning a REAL stand-in harness, signalled from outside.
//!
//! The stand-in is a python script found as `quecto` on the TUI's PATH. It
//! writes its pid, announces the protocol and a socket the scenario serves
//! (accepting, draining, answering nothing), logs every SIGTERM it receives
//! and exits 0 on the first. What the scenario reads afterwards is evidence:
//! the pid file, the signal log, the TUI's exit status.

use crate::TuiWorld;
use cucumber::{given, then, when};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct OwnerSignalsFixture {
    dir: tempfile::TempDir,
    tui: std::process::Child,
    /// Bytes arrived on the served socket: the TUI's loop is running.
    loop_started: Arc<AtomicBool>,
    signalled_at: Option<Instant>,
    /// The same moment on the wall clock, for the stand-in's own log.
    signalled_at_unix: Option<f64>,
    tui_status: Option<std::process::ExitStatus>,
}

impl OwnerSignalsFixture {
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }
    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.path(name))
            .ok()
            .map(|s| s.trim().to_string())
    }
    fn harness_pid(&self) -> Option<i32> {
        self.read("harness.pid")?.parse().ok()
    }
}

impl Drop for OwnerSignalsFixture {
    fn drop(&mut self) {
        // Scenario cleanup only: the detach scenario leaves the stand-in
        // alive on purpose, and a failed scenario may leave both.
        let _ = self.tui.kill();
        let _ = self.tui.wait();
        if let Some(pid) = self.harness_pid() {
            // SAFETY: a pid this scenario's stand-in wrote for itself.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes liveness of a pid this scenario spawned.
    let probed = unsafe { libc::kill(pid, 0) == 0 };
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| s.get(s.rfind(')')? + 2..)?.chars().next());
    probed && state.is_some_and(|st| st != 'Z' && st != 'X')
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs_f64()
}

/// The stand-in's own log: the wall-clock second of each SIGTERM it took.
fn term_times(f: &OwnerSignalsFixture) -> Vec<f64> {
    f.read("harness.signals")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("TERM ")?.trim().parse().ok())
        .collect()
}

fn wait_until(deadline: Instant, what: &str, mut done: impl FnMut() -> bool) {
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

const STAND_IN: &str = r#"#!/usr/bin/env python3
import os, signal, sys, time
d = os.environ["QUECTO_BDD_DIR"]
open(d + "/harness.pid", "w").write(str(os.getpid()))
def on_term(*_):
    open(d + "/harness.signals", "a").write("TERM %.3f\n" % time.time())
    sys.exit(0)
signal.signal(signal.SIGTERM, on_term)
time.sleep(float(os.environ.get("QUECTO_BDD_ANNOUNCE_DELAY", "0")))
print("quecto-agent-protocol: 2", file=sys.stderr, flush=True)
print("quecto-agent-socket: " + d + "/agent.sock", file=sys.stderr, flush=True)
while True:
    signal.pause()
"#;

/// Serve `sock`: accept every connection and drain it, flagging the first
/// byte (the TUI's startup requests, sent from inside its loop).
fn serve(sock: &std::path::Path, loop_started: Arc<AtomicBool>) {
    let listener = std::os::unix::net::UnixListener::bind(sock).expect("bind stand-in socket");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let loop_started = Arc::clone(&loop_started);
            std::thread::spawn(move || {
                use std::io::Read;
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    loop_started.store(true, Ordering::SeqCst);
                }
            });
        }
    });
}

fn start(world: &mut TuiWorld, tui_args: &[&str], announce_delay_secs: u32, with_stand_in: bool) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("fixture dir");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(dir.path().join("home")).unwrap();
    if with_stand_in {
        let script = bin.join("quecto");
        std::fs::write(&script, STAND_IN).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let loop_started = Arc::new(AtomicBool::new(false));
    serve(&dir.path().join("agent.sock"), Arc::clone(&loop_started));
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let stderr = std::fs::File::create(dir.path().join("tui.err")).unwrap();
    let tui = std::process::Command::new(env!("CARGO_BIN_EXE_quecto-tui"))
        .args(tui_args)
        .env("PATH", path)
        .env("HOME", dir.path().join("home"))
        .env("XDG_CONFIG_HOME", dir.path().join("home"))
        .env("QUECTO_BASE_DIR", dir.path().join("base"))
        .env("QUECTO_BDD_DIR", dir.path())
        .env("QUECTO_BDD_ANNOUNCE_DELAY", announce_delay_secs.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr)
        .spawn()
        .expect("spawn the real quecto-tui");
    world.tui_owner_signals = Some(OwnerSignalsFixture {
        dir,
        tui,
        loop_started,
        signalled_at: None,
        signalled_at_unix: None,
        tui_status: None,
    });
}

fn fixture(world: &mut TuiWorld) -> &mut OwnerSignalsFixture {
    world
        .tui_owner_signals
        .as_mut()
        .expect("owner-signals fixture")
}

fn wait_for_loop(world: &mut TuiWorld) {
    let f = fixture(world);
    let started = Arc::clone(&f.loop_started);
    wait_until(
        Instant::now() + Duration::from_secs(20),
        "the TUI's loop to send its startup requests",
        || started.load(Ordering::SeqCst),
    );
}

fn wait_for_harness_pid(world: &mut TuiWorld) {
    let f = fixture(world);
    let pid_file = f.path("harness.pid");
    wait_until(
        Instant::now() + Duration::from_secs(20),
        "the stand-in harness pid",
        || pid_file.exists(),
    );
}

// ── Given ──────────────────────────────────────────────────────────────────

#[given("a real TUI process owns a stand-in harness")]
fn owns_stand_in(world: &mut TuiWorld) {
    start(world, &[], 0, true);
    wait_for_harness_pid(world);
    wait_for_loop(world);
}

#[given("a real TUI process owns a stand-in harness started with --detach-on-exit")]
fn owns_stand_in_detached(world: &mut TuiWorld) {
    start(world, &["--detach-on-exit"], 0, true);
    wait_for_harness_pid(world);
    wait_for_loop(world);
}

#[given(
    regex = r"^a real TUI process is starting a stand-in harness( with --detach-on-exit)? that announces its socket after (\d+) seconds$"
)]
fn starting_stand_in(world: &mut TuiWorld, detach: String, delay: u32) {
    let args: &[&str] = if detach.is_empty() {
        &[]
    } else {
        &["--detach-on-exit"]
    };
    start(world, args, delay, true);
    wait_for_harness_pid(world);
    // The announcement is still pending: the TUI is in its startup window.
    assert!(!fixture(world).loop_started.load(Ordering::SeqCst));
}

#[given("a real TUI process is attached to a socket served by the scenario")]
fn attached(world: &mut TuiWorld) {
    let dir = tempfile::tempdir().expect("socket dir");
    let sock = dir.path().join("agent.sock");
    world._extra_temp_dirs.push(dir);
    let sock = sock.to_str().unwrap().to_string();
    start(world, &["--socket", &sock], 0, false);
    // The scenario serves the announced-by-flag socket too.
    let f = fixture(world);
    serve(std::path::Path::new(&sock), Arc::clone(&f.loop_started));
    wait_for_loop(world);
}

// ── When ───────────────────────────────────────────────────────────────────

#[when(regex = r"^the TUI process receives (SIGHUP|SIGTERM|SIGINT|SIGKILL)$")]
fn receives(world: &mut TuiWorld, signal: String) {
    let f = fixture(world);
    let sig = match signal.as_str() {
        "SIGHUP" => libc::SIGHUP,
        "SIGTERM" => libc::SIGTERM,
        "SIGINT" => libc::SIGINT,
        _ => libc::SIGKILL,
    };
    let pid = i32::try_from(f.tui.id()).unwrap();
    f.signalled_at = Some(Instant::now());
    f.signalled_at_unix = Some(unix_now());
    // SAFETY: the TUI process this scenario spawned.
    unsafe {
        libc::kill(pid, sig);
    }
}

fn tui_status(world: &mut TuiWorld) -> std::process::ExitStatus {
    let f = fixture(world);
    if let Some(status) = f.tui_status {
        return status;
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = f.tui.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the TUI process did not exit: {:?}",
            f.read("tui.err")
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    f.tui_status = Some(status);
    status
}

// ── Then ───────────────────────────────────────────────────────────────────

#[then(regex = r"^the stand-in harness should have exited on one SIGTERM within (\d+) seconds$")]
fn harness_exited_on_term(world: &mut TuiWorld, within: u64) {
    let f = fixture(world);
    let pid = f.harness_pid().expect("the stand-in wrote its pid");
    let deadline = f.signalled_at.expect("signalled") + Duration::from_secs(within);
    wait_until(deadline, "the stand-in harness to exit", || {
        !process_alive(pid)
    });
    assert_eq!(
        term_times(f).len(),
        1,
        "exactly one SIGTERM reached the stand-in: {:?}",
        f.read("harness.signals")
    );
}

/// Promptness, judged by the stand-in's own clock (the scenario runner may
/// schedule this step late): its SIGTERM came within `within` seconds of
/// the TUI being signalled.
#[then(
    regex = r"^the stand-in harness should have received its SIGTERM within (\d+) seconds of the signal$"
)]
fn harness_term_prompt(world: &mut TuiWorld, within: f64) {
    let f = fixture(world);
    let pid = f.harness_pid().expect("the stand-in wrote its pid");
    wait_until(
        Instant::now() + Duration::from_secs(20),
        "the stand-in harness to exit",
        || !process_alive(pid),
    );
    let sent = f.signalled_at_unix.expect("signalled");
    let times = term_times(f);
    assert_eq!(times.len(), 1, "exactly one SIGTERM: {times:?}");
    let after = times[0] - sent;
    assert!(
        (-0.5..=within).contains(&after),
        "the stand-in took its SIGTERM {after:.3} s after the TUI was signalled"
    );
}

#[then(regex = r"^the stand-in harness should still be running (\d+) seconds later$")]
fn harness_still_running(world: &mut TuiWorld, secs: u64) {
    let f = fixture(world);
    let pid = f.harness_pid().expect("the stand-in wrote its pid");
    std::thread::sleep(Duration::from_secs(secs));
    assert!(
        process_alive(pid),
        "the detached stand-in must outlive the TUI"
    );
    assert_eq!(f.read("harness.signals"), None, "nothing signalled it");
}

#[then("no stand-in harness was ever started")]
fn no_harness(world: &mut TuiWorld) {
    let _ = tui_status(world);
    assert_eq!(fixture(world).harness_pid(), None);
}

#[then(regex = r"^the TUI process should have exited with code (\d+)$")]
fn tui_exit_code(world: &mut TuiWorld, code: i32) {
    let status = tui_status(world);
    assert_eq!(
        status.code(),
        Some(code),
        "TUI status {status:?}; stderr: {:?}",
        fixture(world).read("tui.err")
    );
}

#[then(regex = r"^the TUI process should have been killed by signal (\d+)$")]
fn tui_killed_by(world: &mut TuiWorld, signal: i32) {
    use std::os::unix::process::ExitStatusExt;
    let status = tui_status(world);
    assert_eq!(status.signal(), Some(signal), "TUI status {status:?}");
}
