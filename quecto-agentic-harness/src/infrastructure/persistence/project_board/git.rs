//! The one way the board store runs git: an absolute program, an
//! allowlisted environment, hooks and background maintenance off, no
//! prompts, its own session, and one deadline for the process and its
//! output. Blocking: callers run it off the async workers
//! (`spawn_blocking`).
//!
//! The repository's own configuration is trusted, as git trusts it: a
//! repo-local `core.sshCommand`, credential helper or remote URL runs as
//! configured. Turning hooks, the filesystem monitor and maintenance off
//! keeps board operations free of side effects in the checkout; it is not
//! a security boundary against the repository's owner.
use crate::domain::project_board::entities::claim::Identity;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// The only variables a board git process inherits: where to find
/// programs, the user's configuration and credential helpers, SSH, and an
/// HTTPS proxy and CA bundle (so it works inside a container). `GH_TOKEN`
/// reaches only a credential helper that reads it (`gh auth
/// git-credential`); git itself never sends it. Everything else,
/// `GIT_DIR`, `GIT_INDEX_FILE` and `GIT_WORK_TREE` included, is dropped.
pub const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "SSH_AUTH_SOCK",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
    "GH_TOKEN",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];
/// Settings every call carries: no hooks, no filesystem monitor, no
/// automatic gc or maintenance.
const SETTINGS: &[&str] = &[
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "gc.auto=0",
    "-c",
    "maintenance.auto=false",
];
pub const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);

/// Why a git call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    NotFound,
    /// An identity git would not commit exactly as given.
    Identity(String),
    Spawn(String),
    TimedOut {
        command: String,
    },
    Failed {
        command: String,
        stderr: String,
    },
}

/// The allowlisted variables of an environment, by [`INHERITED`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitEnvironment(Vec<(OsString, OsString)>);

impl GitEnvironment {
    pub fn allowlisted(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        let kept = vars
            .into_iter()
            .filter(|(key, _)| INHERITED.iter().any(|name| OsStr::new(name) == key));
        Self(kept.collect())
    }

    /// This process's environment, allowlisted.
    pub fn of_process() -> Self {
        Self::allowlisted(std::env::vars_os())
    }

    fn get(&self, key: &str) -> Option<&OsStr> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_os_str())
    }
}

/// git for one repository, run as one committer.
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
    repo: PathBuf,
    env: GitEnvironment,
    identity: Identity,
    pub(super) local_timeout: Duration,
    pub(super) network_timeout: Duration,
}

/// What a finished git call printed.
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Git {
    /// git as found on the environment's own `PATH`, absolute, committing
    /// as `identity`: a valid identity that git records byte for byte.
    ///
    /// # Errors
    /// No git on the `PATH`, or an identity git would alter.
    pub fn new(repo: &Path, env: GitEnvironment, identity: Identity) -> Result<Self, GitError> {
        identity
            .validate("identity")
            .map_err(|error| GitError::Identity(error.to_string()))?;
        if !kept_by_git(&identity.name) || !kept_by_git(&identity.email) {
            return Err(GitError::Identity(format!(
                "{identity:?}: git trims punctuation and spaces from the ends of a name or email, \
                 so it must start and end with a letter, digit or one of ()[]{{}}_-+*&^%$#@!?~=|/"
            )));
        }
        let program = env
            .get("PATH")
            .and_then(|path| on_path(path, "git"))
            .ok_or(GitError::NotFound)?;
        debug_assert!(program.is_absolute());
        let (local_timeout, network_timeout) = (LOCAL_TIMEOUT, NETWORK_TIMEOUT);
        Ok(Self {
            program,
            repo: repo.to_path_buf(),
            env,
            identity,
            local_timeout,
            network_timeout,
        })
    }

    /// A local call that must succeed; its standard output.
    pub fn ok(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
        let output = self.run(args, input, self.local_timeout)?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(failed(args, &output))
        }
    }

    /// The committer every write is made as.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// A local call's trimmed text output.
    pub fn text(&self, args: &[&str], input: Option<&[u8]>) -> Result<String, GitError> {
        Ok(String::from_utf8_lossy(&self.ok(args, input)?)
            .trim_end()
            .to_string())
    }

    /// A call that reaches the remote, whatever its exit status.
    pub fn network(&self, args: &[&str]) -> Result<Output, GitError> {
        self.run(args, None, self.network_timeout)
    }

    pub fn run(
        &self,
        args: &[&str],
        input: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<Output, GitError> {
        let mut command = Command::new(&self.program);
        command
            .arg("-C")
            .arg(&self.repo)
            .args(SETTINGS)
            .args(args)
            .env_clear()
            .envs(self.env.0.iter().map(|(key, value)| (key, value)))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", &self.identity.name)
            .env("GIT_AUTHOR_EMAIL", &self.identity.email)
            .env("GIT_COMMITTER_NAME", &self.identity.name)
            .env("GIT_COMMITTER_EMAIL", &self.identity.email)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Its own session, so no ssh or helper it starts can reach the
        // terminal quecto runs in, and its own process group, so the
        // deadline ends git and everything it started.
        // setsid(2) is async-signal-safe and touches no memory of the
        // parent; the closure allocates nothing.
        // SAFETY: the pre_exec closure only calls setsid.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut command, || match libc::setsid() {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            });
        }
        let mut child = command
            .spawn()
            .map_err(|error| GitError::Spawn(error.to_string()))?;
        if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
            let bytes = bytes.to_vec();
            thread::spawn(move || stdin.write_all(&bytes));
        }
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let deadline = Instant::now() + timeout;
        let timed_out = || GitError::TimedOut {
            command: args.join(" "),
        };
        // Watch for git's exit without reaping it, so its group cannot be
        // reused before the group is ended.
        let exited = loop {
            match exited_unreaped(child.id()) {
                Ok(true) => break true,
                Ok(false) if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
                Ok(false) => break false,
                Err(error) => {
                    end_group(child.id());
                    let _ = child.wait();
                    return Err(GitError::Spawn(error.to_string()));
                }
            }
        };
        // Whatever git started and left behind (a credential helper, an
        // ssh master) ends with it, closing the pipes it may hold.
        end_group(child.id());
        let status = child
            .wait()
            .map_err(|error| GitError::Spawn(error.to_string()))?;
        if !exited {
            return Err(timed_out());
        }
        let left = |at: Instant| at.saturating_duration_since(Instant::now());
        let stdout = stdout
            .recv_timeout(left(deadline))
            .map_err(|_| timed_out())?;
        let stderr = stderr
            .recv_timeout(left(deadline))
            .map_err(|_| timed_out())?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
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
/// leader): git and whatever it started. The leader is not yet reaped, so
/// the group is still ours.
fn end_group(leader: u32) {
    if let Ok(group) = i32::try_from(leader) {
        // SAFETY: kill(2) takes plain integers and touches no memory.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
    }
}

/// Reads a pipe to its end on its own thread; the bytes arrive on the
/// channel, so the caller can stop waiting at its deadline.
fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        let _ = sender.send(bytes);
    });
    receiver
}

/// Whether git records `text` as given: it trims spaces and some
/// punctuation from the ends of a name or email, so each end must be one
/// of the characters it keeps.
fn kept_by_git(text: &str) -> bool {
    let kept = |c: char| c.is_alphanumeric() || "()[]{}_-+*&^%$#@!?~=|/".contains(c);
    text.chars().next().is_some_and(kept) && text.chars().next_back().is_some_and(kept)
}

pub fn failed(args: &[&str], output: &Output) -> GitError {
    GitError::Failed {
        command: args.join(" "),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    }
}

/// The first executable regular file named `program` in an absolute `PATH`
/// directory.
fn on_path(path: &OsStr, program: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|metadata| {
                use std::os::unix::fs::PermissionsExt;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}
