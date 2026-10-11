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
use super::own_group::{Ended, end_group, exited_unreaped};
use crate::infrastructure::processes::parent_death_signal::{
    SPAWN_ARMS, own_session_dying_with_us,
};
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

/// Why a gh run ended without its status: something else reaped gh.
const REAPED_ELSEWHERE: &str = "gh was reaped elsewhere; its group was left alone";

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
        let host = repo
            .split('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let configured = vars
            .iter()
            .find(|(key, _)| key == "GH_HOST")
            .and_then(|(_, value)| value.to_str())
            .map(str::to_ascii_lowercase);
        let enterprise = matches!(
            configured.as_deref(),
            Some(named) if named == host && host != "github.com"
        );
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
        // A session of its own (nothing gh starts reaches quecto's
        // terminal, and its whole group can be ended), sent SIGTERM if quecto
        // dies (on Linux): this thread forks gh and waits for it.
        own_session_dying_with_us(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("starting gh ({SPAWN_ARMS}): {error}"))?;
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
                let unread = format!("gh's output could not be read: {error}");
                return match end_group(child.id()) {
                    Ended::Signalled => {
                        let _ = child.wait();
                        Err(unread)
                    }
                    Ended::NotOurs => Err(format!("{unread}; {REAPED_ELSEWHERE}")),
                };
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
                // Whether gh is still our unreaped child is unknown here
                // (ECHILD: something else reaped it, and its pid, and so
                // its group's, may already be reused), so nothing is
                // signalled and nothing waited on. Unreachable while this
                // process leaves SIGCHLD alone, which it does.
                Err(error) => return Err(format!("watching gh: {error}")),
            }
        };
        // Whatever gh left behind holding its pipes ends with it, so the
        // pipes reach their end (what gh wrote stays readable) and no pump
        // thread outlives the call.
        match end_group(child.id()) {
            Ended::Signalled => {}
            // Something else reaped gh (unreachable while this process
            // leaves SIGCHLD alone): nothing to wait on.
            Ended::NotOurs => return Err(REAPED_ELSEWHERE.to_string()),
        }
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

/// The `host/owner/name` a remote URL names when its host is GitHub's
/// (`github.com`) or the enterprise host `configured_host` names
/// (`GH_HOST`), over https or ssh; any user in the URL is dropped, never
/// passed on. `None` for any other host.
pub fn github_repository(remote_url: &str, configured_host: Option<&str>) -> Option<String> {
    let url = remote_url.trim();
    let (authority, path) = match url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("ssh://"))
    {
        Some(rest) => rest.split_once('/')?,
        // git's scp-like `[user@]host:owner/name`.
        None if !url.contains("://") => url.split_once(':')?,
        None => return None,
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.split(':').next()?.to_ascii_lowercase();
    let configured = configured_host.map(str::to_ascii_lowercase);
    let known = host == "github.com" || configured.as_deref() == Some(host.as_str());
    if !known {
        return None;
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let repo = format!("{host}/{path}");
    repository_name(&repo).then_some(repo)
}

/// `host/owner/name`: a host name; an owner of letters, digits, `-` and
/// `_` (an enterprise-managed user's `name_short`) starting with a letter
/// or digit; and a name of letters, digits, `-`,
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
    let owner = |text: &str| made_of(text, b"-_") && starts(text, &|b| b.is_ascii_alphanumeric());
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
