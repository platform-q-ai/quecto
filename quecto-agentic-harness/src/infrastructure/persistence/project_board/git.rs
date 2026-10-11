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
use crate::domain::project_board::value_objects::shown_text::shown_char;
use crate::domain::redaction::{redact_secrets, redact_url_userinfo};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// The only variables a board git process inherits: where to find
/// programs, the user's configuration and credential helpers, SSH, the
/// GitHub CLI's token and configuration, and proxies and a CA bundle (so it
/// works inside a container). Every program git starts (a credential
/// helper, `ssh`, a remote helper, a configured `core.sshCommand`) inherits
/// them in turn, `GH_TOKEN` and `GITHUB_TOKEN` included; git itself sends
/// neither, only a credential helper that reads one does.
/// `GIT_CEILING_DIRECTORIES` passes too: it only narrows where git looks
/// for a repository. Everything else,
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
    "GIT_CEILING_DIRECTORIES",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_CONFIG_DIR",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
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
/// The most standard output a call takes unless it names its own bound:
/// a line of plumbing, a path, a commit's committer.
pub const SHORT_OUTPUT: usize = 64 * 1024;
/// The most standard error kept from any call.
pub const MAX_STDERR: usize = 16 * 1024;
/// Standard error read past [`MAX_STDERR`] so a secret the cap would cut
/// is still whole when it is redacted; the margin is then dropped.
const STDERR_MARGIN: usize = 4 * 1024;
/// The most characters of git's own message an error quotes.
pub const MAX_QUOTED_CHARS: usize = 2 * 1024;
/// How long an exited git's output may take to drain, past its deadline
/// if need be: its process group is gone, so only something outside it
/// can still hold its pipes.
pub const DRAIN_GRACE: Duration = Duration::from_secs(1);
/// What git says when it refuses an identity: "name consists only of
/// disallowed characters", "empty ident name not allowed", "unable to
/// auto-detect email address".
const IDENTITY_REFUSALS: &[&str] = &["disallowed characters", "ident name", "email address"];
/// The first and the longest pause between looks at whether git exited.
const FIRST_POLL: Duration = Duration::from_millis(1);
const LONGEST_POLL: Duration = Duration::from_millis(25);

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
    /// git printed more than the call takes.
    TooMuchOutput {
        command: String,
        limit: usize,
    },
    Failed {
        command: String,
        /// git's message, bounded and stripped of control characters.
        stderr: String,
    },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("no git on the PATH"),
            Self::Identity(why) => f.write_str(why),
            Self::Spawn(why) => write!(f, "git could not run: {why}"),
            Self::TimedOut { command } => write!(f, "git {command} timed out"),
            Self::TooMuchOutput { command, limit } => {
                write!(f, "git {command} printed more than {limit} bytes")
            }
            Self::Failed { command, stderr } => write!(f, "git {command} failed: {stderr}"),
        }
    }
}

/// Whether a call may reach the remote. A local call never fetches an
/// object on its own (`GIT_NO_LAZY_FETCH`): in a partial clone, the store
/// fetches what a board needs in one call instead of one per object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Local,
    Network,
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
    /// as `identity`: a valid identity that this git records byte for
    /// byte, which git itself is asked (`git var GIT_COMMITTER_IDENT`).
    ///
    /// # Errors
    /// No git on the `PATH`, git cannot run in `repo`, or an identity git
    /// would alter.
    pub fn new(repo: &Path, env: GitEnvironment, identity: Identity) -> Result<Self, GitError> {
        identity
            .validate("identity")
            .map_err(|error| GitError::Identity(error.to_string()))?;
        let program = env
            .get("PATH")
            .and_then(|path| on_path(path, "git"))
            .ok_or(GitError::NotFound)?;
        debug_assert!(program.is_absolute());
        let git = Self {
            program,
            repo: repo.to_path_buf(),
            env,
            identity,
            local_timeout: LOCAL_TIMEOUT,
            network_timeout: NETWORK_TIMEOUT,
        };
        git.records_identity_as_given()?;
        Ok(git)
    }

    /// git trims some characters from the ends of a name or email and
    /// drops others inside, by rules that differ between its versions, so
    /// the installed git is asked what it would record.
    fn records_identity_as_given(&self) -> Result<(), GitError> {
        let refused = |why: String| {
            GitError::Identity(format!(
                "{:?} <{:?}>: {why}; give the name and email exactly as git records them",
                self.identity.name, self.identity.email
            ))
        };
        // Only what git says of the identity refuses it; any other failure
        // (git cannot run in the repository, say) is that failure.
        let about_identity =
            |stderr: &str| IDENTITY_REFUSALS.iter().any(|said| stderr.contains(said));
        let ident = match self.text(&["var", "GIT_COMMITTER_IDENT"], None) {
            Ok(ident) => ident,
            Err(GitError::Failed { stderr, .. }) if about_identity(&stderr) => {
                return Err(refused(stderr));
            }
            Err(error) => return Err(error),
        };
        match recorded_identity(&ident) {
            Some((name, email)) if name == self.identity.name && email == self.identity.email => {
                Ok(())
            }
            Some(recorded) => Err(refused(format!("git records it as {recorded:?}"))),
            None => Err(refused(format!(
                "git var printed {:?}",
                quoted(ident.as_bytes())
            ))),
        }
    }

    /// A local call that must succeed; its standard output, at most
    /// [`SHORT_OUTPUT`] bytes.
    pub fn ok(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, GitError> {
        self.ok_up_to(args, input, SHORT_OUTPUT)
    }

    /// A local call that must succeed; its standard output, at most `limit`
    /// bytes.
    pub fn ok_up_to(
        &self,
        args: &[&str],
        input: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<u8>, GitError> {
        let output = self.local(args, input, limit)?;
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

    /// The repository's common git directory, shared by all its worktrees.
    ///
    /// # Errors
    /// git cannot name it.
    pub fn common_dir(&self) -> Result<PathBuf, GitError> {
        let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
        let bytes = self.ok(&args, None)?;
        let text = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(OsStr::from_bytes(text)))
    }

    /// A local call, whatever its exit status.
    pub fn local(
        &self,
        args: &[&str],
        input: Option<&[u8]>,
        limit: usize,
    ) -> Result<Output, GitError> {
        self.run(args, input, limit, Reach::Local)
    }

    /// A call that reaches the remote, whatever its exit status.
    pub fn network(&self, args: &[&str]) -> Result<Output, GitError> {
        self.network_with(args, None, SHORT_OUTPUT)
    }

    /// A call that reaches the remote with `input`, whatever its exit
    /// status, taking at most `limit` bytes of output.
    pub fn network_with(
        &self,
        args: &[&str],
        input: Option<&[u8]>,
        limit: usize,
    ) -> Result<Output, GitError> {
        self.run(args, input, limit, Reach::Network)
    }

    fn run(
        &self,
        args: &[&str],
        input: Option<&[u8]>,
        limit: usize,
        reach: Reach,
    ) -> Result<Output, GitError> {
        let timeout = match reach {
            Reach::Local => self.local_timeout,
            Reach::Network => self.network_timeout,
        };
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
        match reach {
            Reach::Local => {
                command.env("GIT_NO_LAZY_FETCH", "1");
            }
            Reach::Network => {}
        }
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
        let stdout = drain(child.stdout.take(), limit);
        let stderr = drain(child.stderr.take(), MAX_STDERR + STDERR_MARGIN);
        let deadline = Instant::now() + timeout;
        let command_line = || args.join(" ");
        // Watch for git's exit without reaping it, so its group cannot be
        // reused before the group is ended.
        let exited = match watch(child.id(), deadline) {
            Ok(exited) => exited,
            // Whether git is still our unreaped child is unknown here
            // (ECHILD: something else reaped it, and its pid, and so its
            // group's, may already be reused), so nothing is signalled and
            // nothing waited on. Unreachable while this process leaves
            // SIGCHLD alone, which it does.
            Err(error) => return Err(GitError::Spawn(format!("watching git: {error}"))),
        };
        // Whatever git started and left behind (a credential helper, an
        // ssh master) ends with it, closing the pipes it may hold.
        end_group(child.id());
        let status = child
            .wait()
            .map_err(|error| GitError::Spawn(error.to_string()))?;
        if !exited {
            return Err(GitError::TimedOut {
                command: command_line(),
            });
        }
        // git exited, and its group is gone: its output drains within a
        // short grace, never the rest of its deadline, however long
        // something outside the group holds the pipe.
        let drained_by = Instant::now() + DRAIN_GRACE;
        let left = || drained_by.saturating_duration_since(Instant::now());
        let timed_out = |_| GitError::TimedOut {
            command: command_line(),
        };
        let stdout = stdout.recv_timeout(left()).map_err(timed_out)?;
        let stderr = stderr.recv_timeout(left()).map_err(timed_out)?;
        if stdout.past_limit {
            return Err(GitError::TooMuchOutput {
                command: command_line(),
                limit,
            });
        }
        let stderr = redacted_stderr(&stderr.bytes, stderr.past_limit);
        debug_assert!(stdout.bytes.len() <= limit && stderr.len() <= MAX_STDERR);
        Ok(Output {
            status,
            stdout: stdout.bytes,
            stderr,
        })
    }
}

/// Watches our unreaped child `pid` until it exits (`true`) or `deadline`
/// passes (`false`), looking again after a pause that doubles up to
/// [`LONGEST_POLL`].
fn watch(pid: u32, deadline: Instant) -> std::io::Result<bool> {
    let mut pause = FIRST_POLL;
    loop {
        if exited_unreaped(pid)? {
            return Ok(true);
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        thread::sleep(pause.min(deadline - now));
        pause = (pause * 2).min(LONGEST_POLL);
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

/// What a pipe held, up to its bound.
struct Drained {
    bytes: Vec<u8>,
    past_limit: bool,
}

/// Reads a pipe to its end on its own thread, keeping at most `limit`
/// bytes and discarding the rest, so git never blocks on a full pipe; the
/// bytes arrive on the channel, so the caller can stop waiting at its
/// deadline.
fn drain(pipe: Option<impl Read + Send + 'static>, limit: usize) -> mpsc::Receiver<Drained> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut past_limit = false;
        if let Some(mut pipe) = pipe {
            let wanted = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
            let _ = (&mut pipe).take(wanted).read_to_end(&mut bytes);
            if bytes.len() > limit {
                past_limit = true;
                bytes.truncate(limit);
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            }
        }
        let _ = sender.send(Drained { bytes, past_limit });
    });
    receiver
}

/// The name and email in a `git var GIT_COMMITTER_IDENT` line, `name
/// <email> seconds zone`: git drops `<` and `>` from both, so the first `
/// <` and the next `> ` delimit them.
fn recorded_identity(ident: &str) -> Option<(String, String)> {
    let (name, rest) = ident.split_once(" <")?;
    let (email, _when) = rest.split_once("> ")?;
    Some((name.to_string(), email.to_string()))
}

/// git's standard error with credentials and secret-shaped spans redacted
/// while whole, then cut to [`MAX_STDERR`] bytes (at a character's edge):
/// a secret the cap would cut is never kept in part. When git printed more
/// than was read (`past_limit`), the read itself may have cut a secret, so
/// the raw tail after the last whitespace goes before anything is redacted.
pub(crate) fn redacted_stderr(bytes: &[u8], past_limit: bool) -> Vec<u8> {
    let whole = match past_limit {
        true => {
            let last = bytes.iter().rposition(u8::is_ascii_whitespace);
            &bytes[..last.unwrap_or(0)]
        }
        false => bytes,
    };
    let text = String::from_utf8_lossy(whole);
    let redacted = redact_secrets(&redact_url_userinfo(&text));
    let mut end = redacted.len().min(MAX_STDERR);
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    redacted.as_bytes()[..end].to_vec()
}

/// A name from git (a tree entry's, a path) as text fit to show: escape
/// sequences dropped; letters, marks, numbers, punctuation and symbols
/// kept; anything else (spaces, controls, format characters such as
/// bidirectional overrides, zero-width and tag characters, unassigned code
/// points) one space between them; at most [`MAX_QUOTED_CHARS`]
/// characters. A name is shown as it is, never redacted: a clear names a
/// file exactly as a listing shows it.
pub fn shown_name(bytes: &[u8]) -> String {
    capped(&sanitised(bytes))
}

/// git's message as text fit to quote: sanitised as [`shown_name`] is,
/// then credentials and secret-shaped spans redacted (after the escape
/// sequences are gone, so none splits a secret past the redaction), then
/// capped (after, so the cap never cuts a secret).
pub fn quoted(bytes: &[u8]) -> String {
    capped(&redact_secrets(&redact_url_userinfo(&sanitised(bytes))))
}

/// `bytes` with escape sequences dropped and every run of characters not
/// shown as one space between shown ones.
fn sanitised(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    let (mut out, mut gap) = (String::new(), false);
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // An escape sequence: ESC, `[`, parameters, then a final byte.
            if chars.next_if_eq(&'[').is_some() {
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        if !shown(c) {
            gap = !out.is_empty();
            continue;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        out.push(c);
    }
    out
}

/// At most [`MAX_QUOTED_CHARS`] characters of `text`, marked when cut.
fn capped(text: &str) -> String {
    match text.chars().count() > MAX_QUOTED_CHARS {
        true => text.chars().take(MAX_QUOTED_CHARS).collect::<String>() + " …",
        false => text.to_string(),
    }
}

/// Whether git's character `c` is shown as it is: the board's one
/// allowlist ([`shown_char`]).
fn shown(c: char) -> bool {
    shown_char(c)
}

pub fn failed(args: &[&str], output: &Output) -> GitError {
    GitError::Failed {
        command: args.join(" "),
        stderr: quoted(&output.stderr),
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
