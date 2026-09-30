//! Stand-in processes for the TUI's process tests (#2283: the workspace
//! has no Python, its tests included). A test runs its own test binary as
//! the stand-in: `QUECTO_TUI_STAND_IN=<mode>` in its environment makes the
//! binary act the stand-in on its main thread and never reach the test
//! harness. The unit tests reach [`run_if_asked`] through a start-up
//! constructor (below, Linux only), before libtest starts a thread;
//! `tui_bdd` calls it first in `main`. Off Linux [`command`] and
//! [`write_script`] refuse, and the tests that run a stand-in are
//! Linux-only. Each stand-in is one process that never forks, so a
//! leader's `kill -KILL $kid; wait $kid` reaps it whole.
//!
//! Modes (each reads its settings from `QUECTO_STAND_IN_*` variables):
//!
//! - `signal-log`: writes its pid to `PID`, then appends `TERM`, `INT` or
//!   `HUP` to `LOG` for every such signal, until `LIFETIME` seconds pass
//!   (so a stand-in a failed test left behind ends).
//! - `slow-agent`: exits 0 on SIGTERM; after `DELAY` seconds announces
//!   `quecto-agent-socket: SOCKET` on stderr.
//! - `pdeathsig`: announces its parent-death signal (`pdeathsig=<n>`) and
//!   `SOCKET` on stderr, then waits, its signal dispositions untouched.
//! - `owner-harness`: writes `harness.pid` in `DIR`; after `DELAY` seconds
//!   announces protocol 2 and `DIR/agent.sock`; on SIGTERM appends
//!   `TERM <unix seconds>` to `DIR/harness.signals` and exits 0.
use std::path::Path;

/// The variable naming the stand-in mode.
pub const MODE: &str = "QUECTO_TUI_STAND_IN";

/// `text` as one single-quoted POSIX shell word.
#[cfg(target_os = "linux")]
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The shell words that run this binary as the stand-in `mode` with
/// `settings` (`QUECTO_STAND_IN_<name>` values), for a `sh -c` script.
#[cfg(target_os = "linux")]
pub fn command(mode: &str, settings: &[(&str, &str)]) -> String {
    let exe = std::env::current_exe().expect("the test binary's path");
    let mut words = vec!["env".to_owned(), format!("{MODE}={}", quoted(mode))];
    for (name, value) in settings {
        words.push(format!("QUECTO_STAND_IN_{name}={}", quoted(value)));
    }
    words.push(quoted(&exe.to_string_lossy()));
    words.join(" ")
}

/// Off Linux no constructor turns the unit-test binary into the stand-in:
/// running it would re-run the whole suite, these tests included, without
/// end. So there is no stand-in command off Linux.
#[cfg(not(target_os = "linux"))]
pub fn command(mode: &str, _settings: &[(&str, &str)]) -> String {
    panic!("the stand-ins are Linux-only (the unit tests' constructor is): {mode}")
}

/// Writes an executable script at `path` that runs the stand-in `mode`
/// (its arguments are ignored). The stand-in's command line ends with the
/// script's path, so a test can find it by its own directory.
pub fn write_script(path: &Path, mode: &str, settings: &[(&str, &str)]) {
    super::test_executable::write_executable(
        path,
        format!("#!/bin/sh\nexec {} \"$0\"\n", command(mode, settings)),
    );
}

/// The stand-ins themselves: Linux's (`sigtimedwait`, `prctl`), as are
/// the tests that run them.
#[cfg(target_os = "linux")]
mod linux {
    use std::io::Write;
    use std::path::Path;
    use std::time::Duration;

    fn setting(name: &str) -> String {
        std::env::var(format!("QUECTO_STAND_IN_{name}"))
            .unwrap_or_else(|_| fail(&format!("QUECTO_STAND_IN_{name} is set")))
    }

    pub(super) fn fail(what: &str) -> ! {
        eprintln!("stand-in: {what}");
        std::process::exit(97)
    }

    /// The signal set `signals`, blocked on this (the only) thread, so they
    /// are taken by `sigwait`/`sigtimedwait` and never by a default action.
    fn blocked(signals: &[i32]) -> libc::sigset_t {
        // SAFETY: an all-zero sigset_t is valid storage for `sigemptyset`.
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: `set` is valid for writes; the signals are valid numbers.
        unsafe {
            libc::sigemptyset(&mut set);
            for signal in signals {
                libc::sigaddset(&mut set, *signal);
            }
        }
        // SAFETY: `set` is initialised; the old mask is not wanted.
        let masked = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
        if masked != 0 {
            fail("block the signals");
        }
        set
    }

    /// The next of `set`'s signals, or `None` when `within` passes first.
    fn next_signal(set: &libc::sigset_t, within: Option<Duration>) -> Option<i32> {
        loop {
            let taken = match within {
                Some(within) => {
                    let timeout = libc::timespec {
                        tv_sec: libc::time_t::try_from(within.as_secs())
                            .unwrap_or(libc::time_t::MAX),
                        tv_nsec: within.subsec_nanos().into(),
                    };
                    // SAFETY: `set` is initialised; no siginfo is wanted.
                    unsafe { libc::sigtimedwait(set, std::ptr::null_mut(), &timeout) }
                }
                // SAFETY: `set` is initialised; no siginfo is wanted.
                None => unsafe { libc::sigwaitinfo(set, std::ptr::null_mut()) },
            };
            match taken {
                signal if signal > 0 => return Some(signal),
                _ => match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::EAGAIN) => return None,
                    Some(libc::EINTR) => {}
                    _ => fail("wait for a signal"),
                },
            }
        }
    }

    fn append(path: &Path, line: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|error| fail(&format!("open {}: {error}", path.display())));
        file.write_all(format!("{line}\n").as_bytes())
            .unwrap_or_else(|error| fail(&format!("append to {}: {error}", path.display())));
    }

    fn announce(line: &str) {
        let mut stderr = std::io::stderr();
        let _written = writeln!(stderr, "{line}").and_then(|()| stderr.flush());
    }

    fn seconds(name: &str) -> Duration {
        let text = setting(name);
        let seconds = text
            .parse()
            .unwrap_or_else(|_| fail(&format!("{name} is seconds")));
        Duration::try_from_secs_f64(seconds).unwrap_or_else(|_| fail(&format!("{name} is seconds")))
    }

    fn delay() -> Duration {
        seconds("DELAY")
    }

    pub(super) fn signal_log() -> ! {
        let set = blocked(&[libc::SIGTERM, libc::SIGINT, libc::SIGHUP]);
        let log = setting("LOG");
        // A bound on how long a stand-in a failed test left behind lives.
        let lifetime = std::time::Instant::now() + seconds("LIFETIME");
        std::fs::write(setting("PID"), std::process::id().to_string())
            .unwrap_or_else(|error| fail(&format!("write the pid: {error}")));
        loop {
            let left = lifetime.saturating_duration_since(std::time::Instant::now());
            let name = match next_signal(&set, Some(left)) {
                Some(libc::SIGTERM) => "TERM",
                Some(libc::SIGINT) => "INT",
                Some(libc::SIGHUP) => "HUP",
                Some(_) => continue,
                None => std::process::exit(0),
            };
            append(Path::new(&log), name);
        }
    }

    pub(super) fn slow_agent() -> ! {
        let set = blocked(&[libc::SIGTERM]);
        if next_signal(&set, Some(delay())).is_some() {
            std::process::exit(0);
        }
        announce(&format!("quecto-agent-socket: {}", setting("SOCKET")));
        let _term = next_signal(&set, None);
        std::process::exit(0)
    }

    pub(super) fn pdeathsig() -> ! {
        let mut signal: libc::c_int = 0;
        // SAFETY: PR_GET_PDEATHSIG writes one c_int through the pointer.
        let read = unsafe { libc::prctl(libc::PR_GET_PDEATHSIG, &mut signal as *mut libc::c_int) };
        if read != 0 {
            fail("read the parent-death signal");
        }
        announce(&format!("pdeathsig={signal}"));
        announce(&format!("quecto-agent-socket: {}", setting("SOCKET")));
        loop {
            // A signal ends the process by its default action.
            // SAFETY: `pause` has no preconditions.
            unsafe { libc::pause() };
        }
    }

    pub(super) fn owner_harness() -> ! {
        let set = blocked(&[libc::SIGTERM]);
        let dir = std::path::PathBuf::from(setting("DIR"));
        std::fs::write(dir.join("harness.pid"), std::process::id().to_string())
            .unwrap_or_else(|error| fail(&format!("write the pid: {error}")));
        let terminated = |dir: &Path| -> ! {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |since| since.as_secs_f64());
            append(&dir.join("harness.signals"), &format!("TERM {now:.3}"));
            std::process::exit(0)
        };
        if next_signal(&set, Some(delay())).is_some() {
            terminated(&dir);
        }
        announce("quecto-agent-protocol: 2");
        announce(&format!(
            "quecto-agent-socket: {}",
            dir.join("agent.sock").display()
        ));
        let _term = next_signal(&set, None);
        terminated(&dir)
    }
}

/// With [`MODE`] set, acts the stand-in and never returns; otherwise
/// returns at once.
pub fn run_if_asked() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    #[cfg(target_os = "linux")]
    match mode.as_str() {
        "signal-log" => linux::signal_log(),
        "slow-agent" => linux::slow_agent(),
        "pdeathsig" => linux::pdeathsig(),
        "owner-harness" => linux::owner_harness(),
        other => linux::fail(&format!("unknown stand-in {other}")),
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("stand-in {mode}: the stand-ins are Linux-only");
        std::process::exit(97)
    }
}

/// The unit-test binary acts a stand-in process when its environment asks
/// (`shell::test_stand_in`), on its main thread before libtest starts: a
/// constructor, run from `.init_array` ahead of `main`.
#[cfg(all(test, target_os = "linux"))]
#[used]
// SAFETY: a function pointer of the C ABI, called once by the loader with no
// arguments it reads; it returns at once unless a stand-in was asked for,
// which ends the process itself.
#[unsafe(link_section = ".init_array")]
static STAND_IN: extern "C" fn() = {
    extern "C" fn stand_in() {
        run_if_asked();
    }
    stand_in
};
