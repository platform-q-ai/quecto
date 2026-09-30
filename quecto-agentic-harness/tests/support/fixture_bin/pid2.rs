//! `pid2-signal-log <command…>`: the signal-logging pid 2 of the #1925 /
//! #1940 in-container proof (`scripts/bdd-in-box/run.sh`; a Python script
//! until #2283). Run as the container's main process under `--init` (so it
//! is pid 2, where a swarm coordinator's harness would sit), it blocks
//! SIGTERM, SIGINT and SIGHUP, runs the command as its child (with them
//! unblocked), and logs every such signal it receives with the sender's
//! pid, uid and command line to `$PID2_SIGNAL_LOG` (default
//! `/home/dev/pid2-signals.log`). It exits with the child's status; the log
//! ends with a `SIGNALS_TO_PID2 <n>` line.
use std::io::Write;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const WATCHED: [i32; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

/// Appends one timestamped line to the log, whole.
struct Log {
    path: String,
    lock: Mutex<()>,
}

impl Log {
    fn line(&self, line: &str) {
        let _held = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |since| since.as_secs_f64());
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| file.write_all(format!("{now:.3} {line}\n").as_bytes()));
        if let Err(error) = appended {
            eprintln!("pid2-signal-log: {}: {error}", self.path);
        }
    }
}

fn watched() -> libc::sigset_t {
    // SAFETY: an all-zero sigset_t is valid storage for `sigemptyset`.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `set` is valid for writes; the signals are valid numbers.
    unsafe {
        libc::sigemptyset(&mut set);
        for signal in WATCHED {
            libc::sigaddset(&mut set, signal);
        }
    }
    set
}

/// `text` as Python's `repr` writes a `str` (the Python wrapper's log
/// shape): single quotes unless it holds one and no double quote, the
/// quote and backslash escaped, control characters as escapes.
fn repr(text: &str) -> String {
    let quote = match (text.contains('\''), text.contains('"')) {
        (true, false) => '"',
        _ => '\'',
    };
    let mut out = String::from(quote);
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if character == quote => {
                out.push('\\');
                out.push(character);
            }
            _ if (character as u32) < 0x20 || character as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", character as u32));
            }
            _ => out.push(character),
        }
    }
    out.push(quote);
    out
}

fn cmdline(pid: i32) -> String {
    match std::fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(bytes) => String::from_utf8_lossy(&bytes)
            .replace('\0', " ")
            .trim()
            .to_owned(),
        Err(error) => format!("<unreadable: {error}>"),
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let (program, rest) = args.split_first().ok_or("pid2-signal-log <command…>")?;
    let set = watched();
    // Blocked before any thread starts, so every thread inherits the mask
    // and only the watcher's `sigwaitinfo` takes them.
    // SAFETY: `set` is initialised; the old mask is not wanted.
    let masked = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
    if masked != 0 {
        return Err("block the watched signals".to_owned());
    }
    let log = Arc::new(Log {
        path: std::env::var("PID2_SIGNAL_LOG")
            .unwrap_or_else(|_| "/home/dev/pid2-signals.log".to_owned()),
        lock: Mutex::new(()),
    });
    // SAFETY: `getppid` has no preconditions and cannot fail.
    let ppid = unsafe { libc::getppid() };
    log.line(&format!(
        "pid2 wrapper started as pid {} ppid {ppid} argv=[{}]",
        std::process::id(),
        args.iter()
            .map(|arg| repr(arg))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let count = Arc::new(AtomicUsize::new(0));
    let (watcher_log, watcher_count) = (Arc::clone(&log), Arc::clone(&count));
    std::thread::spawn(move || {
        loop {
            // SAFETY: an all-zero siginfo_t is valid storage for the call.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: `set` is initialised and `info` is valid for writes.
            let signal = unsafe { libc::sigwaitinfo(&set, &mut info) };
            if signal < 0 {
                continue;
            }
            watcher_count.fetch_add(1, Ordering::SeqCst);
            // A signal `sigwaitinfo` took carries its sender's pid and uid.
            // SAFETY: `info` was filled by `sigwaitinfo` for a kill-type signal.
            let (pid, uid) = unsafe { (info.si_pid(), info.si_uid()) };
            watcher_log.line(&format!(
                "SIGNAL signo={signal} si_pid={pid} si_uid={uid} sender_cmdline={}",
                repr(&cmdline(pid))
            ));
        }
    });
    let mut command = std::process::Command::new(program);
    command.args(rest);
    // The mask is inherited across fork and exec: the child unblocks it, so
    // the suite and every harness it starts see signals normally.
    // The hook only calls `pthread_sigmask`, between fork and exec.
    // SAFETY: `pthread_sigmask` is async-signal-safe, so the hook is sound.
    unsafe {
        command.pre_exec(move || {
            match libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) {
                0 => Ok(()),
                _ => Err(std::io::Error::last_os_error()),
            }
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("start {program}: {error}"))?;
    log.line(&format!("child pid {}", child.id()));
    let status = child.wait().map_err(|error| format!("wait: {error}"))?;
    // Python's `Popen.wait()`: the exit code, or minus the ending signal.
    let code = match (status.code(), status.signal()) {
        (Some(code), _) => code,
        (None, Some(signal)) => -signal,
        (None, None) => return Err(format!("{program} ended with no code or signal")),
    };
    log.line(&format!("child exited status={code}"));
    log.line(&format!("SIGNALS_TO_PID2 {}", count.load(Ordering::SeqCst)));
    let code = match code {
        0.. => code,
        signal => 128 - signal,
    };
    std::process::exit(code)
}
