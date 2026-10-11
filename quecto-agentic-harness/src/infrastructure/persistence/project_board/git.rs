//! The one way the board store runs git: an absolute program, an
//! allowlisted environment, hooks and background maintenance off, no
//! prompts, and a deadline. Blocking: callers run it off the async workers
//! (`spawn_blocking`).
use crate::domain::project_board::entities::claim::Identity;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// The only variables a board git process inherits: where to find
/// programs, the user's configuration and credential helpers, and SSH.
/// Everything else, `GIT_DIR`, `GIT_INDEX_FILE` and `GIT_WORK_TREE`
/// included, is dropped.
pub const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "SSH_AUTH_SOCK",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
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
    Spawn(String),
    TimedOut { command: String },
    Failed { command: String, stderr: String },
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
    /// git as found on the environment's own `PATH`, absolute.
    pub fn new(repo: &Path, env: GitEnvironment, identity: Identity) -> Result<Self, GitError> {
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
        // Its own process group, so a deadline ends git and everything it
        // started (an ssh, a remote helper), never only git.
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
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
        let status = loop {
            match child
                .try_wait()
                .map_err(|error| GitError::Spawn(error.to_string()))?
            {
                Some(status) => break status,
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
                None => {
                    end_group(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(GitError::TimedOut {
                        command: args.join(" "),
                    });
                }
            }
        };
        let joined = |reader: thread::JoinHandle<Vec<u8>>| reader.join().unwrap_or_default();
        Ok(Output {
            status,
            stdout: joined(stdout),
            stderr: joined(stderr),
        })
    }
}

/// Ends the process group `leader` leads (`process_group(0)` made the
/// child its leader): git and whatever it started.
fn end_group(leader: u32) {
    if let Ok(group) = i32::try_from(leader) {
        // The group is this call's own child's.
        // SAFETY: kill(2) takes plain integers and touches no memory.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
    }
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
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
