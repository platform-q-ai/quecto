//! The project board's [`PullRequestStates`] over GitHub (#2477): where a
//! pull request stands, read with the `gh` CLI the user has signed in
//! (`gh pr view <n> --repo <host>/<owner>/<name> --json state`).
//!
//! `gh` runs from an absolute path on the given `PATH`, with an
//! allowlisted environment (the user's `gh` configuration, a token for it,
//! a proxy and CA bundle; the host travels with the repository, never in
//! the environment), no prompts, in a session of its own, and a deadline.
//! Its answer is bounded: at most [`STDOUT_CAPACITY`] bytes of stdout and
//! the last [`STDERR_TAIL_CAPACITY`] bytes of stderr, sanitised as a
//! container script's are. Once `gh` exits (or at the deadline) its
//! process group is ended, so nothing it left behind outlives the call,
//! and its pipes get [`OUTPUT_GRACE`] to deliver what it wrote. Blocking:
//! callers run it off the async workers.
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::application::project_board::ports::{
    PullRequestState, PullRequestStates, PullRequestStatesError,
};
use crate::infrastructure::processes::child_stderr_tail::STDERR_TAIL_CAPACITY;
use crate::infrastructure::processes::containers::script_stderr::sanitise;

/// The variables `gh` inherits: where to find programs, its configuration
/// and token, and a proxy and CA bundle. Everything else is dropped.
pub const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "GH_CONFIG_DIR",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];
/// Tokens for a GitHub Enterprise host: passed on only when the
/// repository's host is not github.com and is the host the environment
/// names in `GH_HOST`, so no token reaches a host it was not meant for.
pub const ENTERPRISE_TOKENS: &[&str] = &["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"];
pub const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// How long `gh`'s pipes get to end once it has exited.
pub const OUTPUT_GRACE: Duration = Duration::from_millis(500);
/// The most of `gh`'s stdout read: a state is one short word.
pub const STDOUT_CAPACITY: usize = 4096;
/// What `gh` says of a pull request number the repository does not have.
const NOT_A_PULL_REQUEST: &str = "Could not resolve to a PullRequest";

/// Pull request states of one GitHub repository, by `gh`.
pub struct GhPullRequestStates {
    program: PathBuf,
    repo: String,
    env: Vec<(OsString, OsString)>,
    timeout: Duration,
}

/// What a finished `gh` printed, bounded.
struct Answer {
    succeeded: bool,
    stdout: String,
    stderr_tail: String,
}

impl GhPullRequestStates {
    /// The states of `repo` (`host/owner/name`), with `gh` found on
    /// `vars`'s own `PATH` and only the [`INHERITED`] variables passed on.
    ///
    /// # Errors
    /// A repository that is not `host/owner/name`, or no `gh` on the
    /// `PATH`.
    pub fn new(
        repo: &str,
        vars: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, PullRequestStatesError> {
        let unavailable = |why: String| PullRequestStatesError::Unavailable(why);
        if !repository_name(repo) {
            return Err(unavailable(format!("{repo:?} is not host/owner/name")));
        }
        let vars: Vec<(OsString, OsString)> = vars.into_iter().collect();
        let host = repo.split('/').next().unwrap_or_default();
        let configured = vars
            .iter()
            .any(|(key, value)| key == "GH_HOST" && value.to_str() == Some(host));
        let enterprise = host != "github.com" && configured;
        let wanted = |key: &OsStr| {
            INHERITED.iter().any(|name| OsStr::new(name) == key)
                || (enterprise && ENTERPRISE_TOKENS.iter().any(|name| OsStr::new(name) == key))
        };
        let env: Vec<(OsString, OsString)> =
            vars.into_iter().filter(|(key, _)| wanted(key)).collect();
        let program = env
            .iter()
            .find(|(key, _)| key == "PATH")
            .and_then(|(_, path)| on_path(path, "gh"))
            .ok_or_else(|| unavailable("no gh on the PATH".into()))?;
        Ok(Self {
            program,
            repo: repo.to_string(),
            env,
            timeout: READ_TIMEOUT,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn run(&self, pr: u64) -> Result<Answer, String> {
        let number = pr.to_string();
        let mut command = Command::new(&self.program);
        command
            .args(["pr", "view", &number, "--repo", &self.repo])
            .args(["--json", "state", "--jq", ".state"])
            .env_clear()
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .envs([
                ("GH_PROMPT_DISABLED", "1"),
                ("GH_NO_UPDATE_NOTIFIER", "1"),
                ("NO_COLOR", "1"),
                ("LC_ALL", "C"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // A session of its own: nothing gh starts reaches quecto's
        // terminal, and its whole group can be ended.
        // setsid(2) is async-signal-safe and touches no memory of the
        // parent; the closure allocates nothing.
        // SAFETY: the pre_exec closure only calls setsid.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut command, || match libc::setsid() {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            });
        }
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let pumps = pump(child.stdout.take(), Keep::Head(STDOUT_CAPACITY)).and_then(|stdout| {
            Ok((
                stdout,
                pump(child.stderr.take(), Keep::Tail(STDERR_TAIL_CAPACITY))?,
            ))
        });
        let (stdout, stderr) = match pumps {
            Ok(pumps) => pumps,
            Err(error) => {
                // No reader for its output: gh is ended rather than left
                // to block on a full pipe.
                end_group(child.id());
                let _ = child.wait();
                return Err(format!("gh's output could not be read: {error}"));
            }
        };
        let deadline = Instant::now() + self.timeout;
        // Watch for gh's exit without reaping it, so its group stays its
        // own until the group is ended.
        let exited = loop {
            match exited_unreaped(child.id()) {
                Ok(true) => break true,
                Ok(false) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Ok(false) => break false,
                Err(error) => {
                    end_group(child.id());
                    let _ = child.wait();
                    return Err(error.to_string());
                }
            }
        };
        // Whatever gh left behind holding its pipes ends with it, so the
        // pipes reach their end (what gh wrote stays readable) and no pump
        // thread outlives the call.
        end_group(child.id());
        let (stdout, stderr) = match exited {
            true => (
                stdout.recv_timeout(OUTPUT_GRACE).unwrap_or_default(),
                stderr.recv_timeout(OUTPUT_GRACE).unwrap_or_default(),
            ),
            false => (Vec::new(), Vec::new()),
        };
        let status = child.wait().map_err(|error| error.to_string())?;
        if !exited {
            return Err(format!("gh did not answer within {:?}", self.timeout));
        }
        Ok(Answer {
            succeeded: status.success(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr_tail: sanitise(&String::from_utf8_lossy(&stderr)),
        })
    }
}

impl PullRequestStates for GhPullRequestStates {
    fn state(&self, pr: u64) -> Result<PullRequestState, PullRequestStatesError> {
        let unavailable = |why: String| PullRequestStatesError::Unavailable(why);
        let answer = self.run(pr).map_err(unavailable)?;
        match (answer.succeeded, answer.stdout.trim()) {
            (true, "OPEN") => Ok(PullRequestState::Open),
            (true, "MERGED") => Ok(PullRequestState::Merged),
            (true, "CLOSED") => Ok(PullRequestState::Closed),
            (true, other) => Err(unavailable(format!(
                "gh read pull request {pr} as {:?}",
                sanitise(other)
            ))),
            (false, _) if answer.stderr_tail.contains(NOT_A_PULL_REQUEST) => {
                Ok(PullRequestState::Unknown)
            }
            (false, _) => Err(unavailable(format!(
                "gh pr view {pr}: {}",
                answer.stderr_tail
            ))),
        }
    }
}

/// `host/owner/name`: a host name; an owner of letters, digits and `-`
/// starting with a letter or digit; and a name of letters, digits, `-`,
/// `_` and `.` (a leading dot too: `.github`), never `.` or `..` and never
/// starting with `-`.
fn repository_name(repo: &str) -> bool {
    let made_of = |text: &str, extra: &[u8]| {
        !text.is_empty()
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
    };
    let starts =
        |text: &str, allowed: &dyn Fn(u8) -> bool| text.bytes().next().is_some_and(allowed);
    let host = |text: &str| {
        made_of(text, b".-") && text.contains('.') && starts(text, &|b| b.is_ascii_alphanumeric())
    };
    let owner = |text: &str| made_of(text, b"-") && starts(text, &|b| b.is_ascii_alphanumeric());
    let name = |text: &str| {
        made_of(text, b"-_.")
            && !matches!(text, "." | "..")
            && starts(text, &|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_')
            })
    };
    matches!(
        repo.split('/').collect::<Vec<_>>().as_slice(),
        [h, o, n] if host(h) && owner(o) && name(n)
    )
}

/// Which part of a pipe's bytes is kept.
#[derive(Clone, Copy)]
enum Keep {
    /// The first so many; the rest is read and dropped.
    Head(usize),
    /// The last so many.
    Tail(usize),
}

/// Reads a pipe to its end on its own thread, keeping a bounded part; the
/// bytes arrive on the channel, so the caller waits only as long as it
/// chooses.
fn pump(
    pipe: Option<impl Read + Send + 'static>,
    keep: Keep,
) -> std::io::Result<mpsc::Receiver<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("gh-output".into())
        .spawn(move || {
            let mut kept: VecDeque<u8> = VecDeque::new();
            if let Some(mut pipe) = pipe {
                let mut buffer = [0u8; 1024];
                loop {
                    match pipe.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            for &byte in &buffer[..read] {
                                match keep {
                                    Keep::Head(most) if kept.len() < most => kept.push_back(byte),
                                    Keep::Head(_) => {}
                                    Keep::Tail(most) => {
                                        if kept.len() == most {
                                            kept.pop_front();
                                        }
                                        kept.push_back(byte);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let _ = sender.send(Vec::from(kept));
        })?;
    Ok(receiver)
}

/// Whether our unreaped child `pid` has exited, observed without reaping
/// it (`WNOWAIT`) or blocking (`WNOHANG`). An interrupted call is "not
/// yet".
fn exited_unreaped(pid: u32) -> std::io::Result<bool> {
    let pid = libc::id_t::from(pid);
    // SAFETY: zero is a valid initial siginfo_t; waitid initializes it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let options = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    // SAFETY: pid is our own unreaped child and info is writable.
    match unsafe { libc::waitid(libc::P_PID, pid, &mut info, options) } {
        // SAFETY: a successful waitid initialized the siginfo union.
        0 => Ok(unsafe { info.si_pid() } != 0),
        _ => {
            let error = std::io::Error::last_os_error();
            match error.kind() {
                std::io::ErrorKind::Interrupted => Ok(false),
                _ => Err(error),
            }
        }
    }
}

/// Ends the process group `leader` leads (`setsid` made the child its
/// leader). The leader is not yet reaped, so the group is still its own.
fn end_group(leader: u32) {
    if let Ok(group) = i32::try_from(leader) {
        // SAFETY: kill(2) takes plain integers and touches no memory.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
    }
}

/// The first executable regular file named `program` in an absolute `PATH`
/// directory.
fn on_path(path: &OsStr, program: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| executable(candidate))
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
#[path = "gh_pull_requests_tests.rs"]
mod tests;
