//! Identity-scoped process ownership for script-managed fixtures (Linux):
//! a fixture script records each process it starts (`track`), the tests
//! count those still alive (`live`), and cleanup ends them (`clean`),
//! signalling a process only through a pidfd opened while its identity
//! (pid and start time) is the one recorded, so a reused pid is never
//! signalled.
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a process may take to exit after each signal.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// A process's start time and state, from `/proc/<pid>/stat`, or `None`
/// when it is gone.
fn identity(pid: i32) -> Option<(String, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, after_name) = stat.rsplit_once(") ")?;
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    Some(((*fields.get(19)?).to_owned(), (*fields.first()?).to_owned()))
}

/// The recorded `[pid, start]` of one tracked process.
fn read_record(path: &Path) -> Option<(i32, String)> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid = i32::try_from(value.get(0)?.as_i64()?).ok()?;
    Some((pid, value.get(1)?.as_str()?.to_owned()))
}

/// The records of `env`, or of every environment.
fn records(root: &Path, env: Option<&str>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            name.ends_with(".json")
                && match env {
                    Some(env) => name.starts_with(&format!("{env}.")),
                    None => true,
                }
        })
        .collect()
}

fn alive(pid: i32, start: &str) -> bool {
    matches!(identity(pid), Some((current, state)) if current == start && state != "Z")
}

fn pidfd_open(pid: i32) -> Option<OwnedFd> {
    // `pidfd_open` takes a pid and flags and answers a new descriptor or -1.
    // SAFETY: no memory is passed, and the descriptor is owned below.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    let fd = i32::try_from(fd).ok().filter(|fd| *fd >= 0)?;
    // SAFETY: `fd` was just returned by `pidfd_open` and nothing else owns it.
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn signal(pidfd: &OwnedFd, signal: i32) -> Result<(), std::io::Error> {
    // A null siginfo and zero flags send `signal` as `kill` would, to the
    // process the open `pidfd` refers to.
    // SAFETY: `pidfd` is open for the call, and no memory is passed.
    let sent = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    match sent {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

/// Whether the process behind `pidfd` exits within `wait`.
fn exits_within(pidfd: &OwnedFd, wait: Duration) -> bool {
    let mut poll = libc::pollfd {
        fd: pidfd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: one valid pollfd, for the duration of the call.
    let ready = unsafe { libc::poll(&mut poll, 1, millis) };
    ready > 0
}

/// Ends the recorded process: SIGTERM, then SIGKILL, each given
/// [`EXIT_WAIT`], through a pidfd opened while its identity matches.
fn terminate(pid: i32, start: &str) -> Result<(), String> {
    let Some(pidfd) = pidfd_open(pid) else {
        return Ok(());
    };
    if !alive(pid, start) {
        return Ok(());
    }
    for sig in [libc::SIGTERM, libc::SIGKILL] {
        match signal(&pidfd, sig) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(()),
            Err(error) => return Err(format!("signal {pid}: {error}")),
        }
        if exits_within(&pidfd, EXIT_WAIT) {
            return Ok(());
        }
    }
    Err(format!("fixture process {pid} did not exit"))
}

/// `track <dir> <env> <pid>`: record `pid` under `env`, one file per
/// launch, written whole (so concurrent joins cannot overwrite each other).
fn track(root: &Path, env: &str, pid: &str) -> Result<(), String> {
    let pid: i32 = pid.parse().map_err(|_| format!("a pid, not {pid}"))?;
    let Some((start, _)) = identity(pid) else {
        return Ok(());
    };
    let record = root.join(format!("{env}.{pid}.json"));
    let staged = root.join(format!("{env}.{pid}.{}.tmp", std::process::id()));
    let text = serde_json::json!([pid, start]).to_string();
    std::fs::write(&staged, text)
        .map_err(|error| format!("write {}: {error}", staged.display()))?;
    std::fs::rename(&staged, &record)
        .map_err(|error| format!("record {}: {error}", record.display()))
}

/// `live <dir> <env>`: how many tracked processes of `env` are still alive
/// (not zombies): evidence of whether members ended before a kill ran.
fn live(root: &Path, env: &str) -> Result<(), String> {
    let count = records(root, Some(env))
        .iter()
        .filter_map(|path| read_record(path))
        .filter(|(pid, start)| alive(*pid, start))
        .count();
    writeln!(std::io::stdout(), "{count}").map_err(|error| format!("print: {error}"))
}

/// `clean <dir> [env]`: end every tracked process (of `env`), dropping
/// its record.
pub fn clean(root: &Path, env: Option<&str>) -> Result<(), String> {
    for path in records(root, env) {
        if let Some((pid, start)) = read_record(&path) {
            terminate(pid, &start)?;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("remove {}: {error}", path.display())),
        }
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let (mode, rest) = args
        .split_first()
        .ok_or("processes track|live|clean <dir> …")?;
    let (directory, rest) = rest.split_first().ok_or("processes: a directory")?;
    let root = Path::new(directory);
    std::fs::create_dir_all(root).map_err(|error| format!("create {directory}: {error}"))?;
    match (mode.as_str(), rest) {
        ("track", [env, pid]) => track(root, env, pid),
        ("live", [env]) => live(root, env),
        ("clean", [env]) => clean(root, Some(env)),
        ("clean", []) => clean(root, None),
        (mode, rest) => Err(format!(
            "processes {mode} {rest:?}: unknown mode or arguments"
        )),
    }
}
