//! The project board's [`PullRequestStates`] over GitHub (#2477): where a
//! pull request stands, read with the `gh` CLI the user has signed in
//! (`gh pr view <n> --repo <owner/repo> --json state`).
//!
//! `gh` runs from an absolute path on the given `PATH`, with an
//! allowlisted environment (the user's `gh` configuration, a token for it,
//! a proxy and CA bundle), no prompts, in a session of its own, and a
//! deadline: at the deadline its whole process group is ended. Blocking:
//! callers run it off the async workers.
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::application::project_board::ports::{
    PullRequestState, PullRequestStates, PullRequestStatesError,
};

/// The variables `gh` inherits: where to find programs, its configuration
/// and token, and a proxy and CA bundle. Everything else is dropped.
pub const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "GH_CONFIG_DIR",
    "GH_HOST",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];
pub const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// What `gh` says of a pull request number the repository does not have.
const NOT_A_PULL_REQUEST: &str = "Could not resolve to a PullRequest";

/// Pull request states of one GitHub repository, by `gh`.
pub struct GhPullRequestStates {
    program: PathBuf,
    repo: String,
    env: Vec<(OsString, OsString)>,
    timeout: Duration,
}

impl GhPullRequestStates {
    /// The states of `repo` (`owner/name`), with `gh` found on `vars`'s own
    /// `PATH` and only the [`INHERITED`] variables passed on.
    ///
    /// # Errors
    /// A repository that is not `owner/name`, or no `gh` on the `PATH`.
    pub fn new(
        repo: &str,
        vars: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, PullRequestStatesError> {
        let unavailable = |why: String| PullRequestStatesError::Unavailable(why);
        if !repository_name(repo) {
            return Err(unavailable(format!("{repo:?} is not owner/name")));
        }
        let env: Vec<(OsString, OsString)> = vars
            .into_iter()
            .filter(|(key, _)| INHERITED.iter().any(|name| OsStr::new(name) == key))
            .collect();
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

    fn run(&self, pr: u64) -> Result<(bool, String, String), String> {
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
        // terminal, and the deadline ends its whole group.
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
        let (stdout, stderr) = (drain(child.stdout.take()), drain(child.stderr.take()));
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait().map_err(|error| error.to_string())? {
                Some(status) => break status,
                None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                None => {
                    // Still unreaped, so the group is still gh's own.
                    end_group(child.id());
                    let _ = child.wait();
                    return Err(format!("gh did not answer within {:?}", self.timeout));
                }
            }
        };
        let left = || deadline.saturating_duration_since(Instant::now());
        let late = |_| format!("gh's output did not end within {:?}", self.timeout);
        let stdout = stdout.recv_timeout(left()).map_err(late)?;
        let stderr = stderr.recv_timeout(left()).map_err(late)?;
        Ok((status.success(), stdout, stderr))
    }
}

impl PullRequestStates for GhPullRequestStates {
    fn state(&self, pr: u64) -> Result<PullRequestState, PullRequestStatesError> {
        let unavailable = |why: String| PullRequestStatesError::Unavailable(why);
        let (succeeded, stdout, stderr) = self.run(pr).map_err(unavailable)?;
        match (succeeded, stdout.trim()) {
            (true, "OPEN") => Ok(PullRequestState::Open),
            (true, "MERGED") => Ok(PullRequestState::Merged),
            (true, "CLOSED") => Ok(PullRequestState::Closed),
            (true, other) => Err(unavailable(format!(
                "gh read pull request {pr} as {other:?}"
            ))),
            (false, _) if stderr.contains(NOT_A_PULL_REQUEST) => Ok(PullRequestState::Unknown),
            (false, _) => Err(unavailable(format!("gh pr view {pr}: {}", stderr.trim()))),
        }
    }
}

/// `owner/name`, each part GitHub's own characters, starting with a letter
/// or digit.
fn repository_name(repo: &str) -> bool {
    let part = |text: &str| {
        !text.is_empty()
            && text
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    matches!(repo.split('/').collect::<Vec<_>>().as_slice(), [owner, name] if part(owner) && part(name))
}

/// Ends the process group `leader` leads (`setsid` made the child its
/// leader).
fn end_group(leader: u32) {
    if let Ok(group) = i32::try_from(leader) {
        // SAFETY: kill(2) takes plain integers and touches no memory.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
    }
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        let _ = sender.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    receiver
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
