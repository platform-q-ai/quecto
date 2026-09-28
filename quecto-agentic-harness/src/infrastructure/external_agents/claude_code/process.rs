//! `claude -p` stream-json as an external agent process (#2286).
//!
//! [`ClaudeCodeLauncher`] starts `claude` with the stream-json flags, the
//! member's allowlisted environment ([`super::environment`]) and its
//! checkout as working directory. The child is spawned through the
//! [`OwnedChildSupervisor`] — the one owner and signaller of every child —
//! in a process group of its own; this module never holds the child and
//! never signals it.
//!
//! [`ClaudeCodeProcess`] writes each user turn as one stream-json user
//! message and decodes stdout through [`super::stream_json`]. Both pipes
//! are pumped on the supervisor's own runtime
//! ([`OwnedChildSupervisor::spawn_pump`]), never the caller's: a runtime
//! the caller drops cannot end the stream or the input of a child that
//! lives on.
//!
//! - **Output.** A line that is not a JSON object (or not UTF-8, or longer
//!   than [`STREAM_LINE_CAP_BYTES`]) is skipped and logged, never the end
//!   of the stream.
//! - **Input.** A writer task owns stdin and writes whole lines taken from
//!   a queue. A turn is queued whole or not at all, so cancelling a send
//!   never leaves half a line on `claude`'s stdin; a queued turn is
//!   written whole even if its sender stops waiting. Closing the input
//!   only closes the queue: it never waits behind a write stuck on a full
//!   pipe. The writer closes stdin once the queued turns are written.
//! - Stderr keeps its last [`EXTERNAL_AGENT_STDERR_TAIL_BYTES`].
//!
//! Closing the input makes `claude` finish and exit 0. Dropping the process
//! closes its input and asks the supervisor to end the child — first by
//! that close, then TERM and KILL to its group if it does not exit within
//! the launcher's [`TerminationBudget`].

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use quecto_line_io::read_bounded_line_into;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{Mutex, mpsc, oneshot};

use super::environment::{MemberEnvironmentError, check_credential, member_environment};
use super::stream_json::StreamJsonDecoder;
use crate::application::external_agent::dto::{
    EXTERNAL_AGENT_STDERR_TAIL_BYTES, ExternalAgentExit, ExternalAgentInputError,
    ExternalAgentLaunchError, ExternalAgentLaunchSpec,
};
use crate::application::external_agent::ports::{
    ExternalAgentLauncher, ExternalAgentProcess, PortFuture,
};
use crate::domain::external_agent::stream::ExternalAgentEvent;
use crate::infrastructure::processes::child_stderr_tail::StderrTail;
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildExit, ChildHandleId, OwnedChildSupervisor, ProcessGroup, ProtocolOutcome,
    TerminationBudget,
};

/// The program looked up on `PATH`.
pub const CLAUDE_PROGRAM: &str = "claude";

/// How the missing program is named in the launch error.
const PROGRAM_NAME: &str = "claude CLI";

/// What the program is required for, in the launch error.
const REQUIRED_FOR: &str = "claude-code members";

/// The longest stdout line decoded; a longer one is skipped and logged.
pub const STREAM_LINE_CAP_BYTES: usize = 16 * 1024 * 1024;

/// Decoded events buffered ahead of the reader; a full buffer holds the
/// pump (and so the child's stdout) until the session reads on.
///
/// Worst case memory: each buffered event comes from one line of at most
/// [`STREAM_LINE_CAP_BYTES`], so a reader that stops reading while
/// `claude` emits only lines at the cap holds up to 256 × 16 MiB = 4 GiB
/// here, plus the pump's one 16 MiB line buffer. Real stream-json lines
/// are KiB (a tool result is the largest), and the session reads every
/// turn to its `result`, so the bound is theoretical; lower the cap or the
/// buffer if a member is ever seen near it.
const EVENT_BUFFER: usize = 256;

/// The `tracing` target of a member process's telemetry.
pub const TELEMETRY_TARGET: &str = "quecto::external_agent";

/// Whole turns queued ahead of the stdin writer; a full queue holds the
/// sender (cancel-safely: an unqueued turn is simply not sent).
const INPUT_QUEUE: usize = 16;

/// Starts `claude` member processes.
pub struct ClaudeCodeLauncher {
    supervisor: Arc<OwnedChildSupervisor>,
    parent_environment: Vec<(OsString, OsString)>,
    termination: TerminationBudget,
}

impl ClaudeCodeLauncher {
    /// A launcher that adopts its children into `supervisor` and builds
    /// each member's environment from `parent_environment` (which is also
    /// where `claude` is looked up: its `PATH`).
    pub fn new(
        supervisor: Arc<OwnedChildSupervisor>,
        parent_environment: Vec<(OsString, OsString)>,
    ) -> Self {
        Self {
            supervisor,
            parent_environment,
            termination: TerminationBudget::DEFAULT,
        }
    }

    /// This launcher, ending a dropped process's child within `budget`
    /// instead of [`TerminationBudget::DEFAULT`].
    pub fn with_termination_budget(mut self, budget: TerminationBudget) -> Self {
        self.termination = budget;
        self
    }

    /// Red stub (#2286): the line cap and buffer budget are not applied yet.
    pub fn with_stream_limits(self, _line_cap: usize, _buffer_bytes: usize) -> Self {
        self
    }

    /// A launcher over this process's own environment.
    pub fn from_process_environment(supervisor: Arc<OwnedChildSupervisor>) -> Self {
        Self::new(supervisor, std::env::vars_os().collect())
    }

    async fn launch(
        &self,
        spec: ExternalAgentLaunchSpec,
    ) -> Result<Box<dyn ExternalAgentProcess>, ExternalAgentLaunchError> {
        // Every check of the spec itself comes before any directory is
        // made or anything is spawned.
        let arguments = claude_arguments(&spec)?;
        check_credential(&spec.credential).map_err(environment_error)?;
        let checkout = checked_checkout(&spec.checkout)?;
        let program = self.resolve_program().ok_or_else(not_found)?;
        let environment =
            member_environment(&self.parent_environment, &spec.member_dir, &spec.credential)
                .map_err(environment_error)?;
        assert!(
            environment.home.is_dir() && environment.config_dir.is_dir(),
            "the member's private directories exist before its child is spawned"
        );
        let mut command = tokio::process::Command::new(&program);
        command
            .args(&arguments)
            .env_clear()
            .envs(
                environment
                    .variables
                    .iter()
                    .map(|(name, value)| (name, value)),
            )
            .current_dir(&checkout)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let spawned = self
            .supervisor
            .spawn(command, ProcessGroup::Own)
            .await
            .map_err(|error| spawn_error(error, &program, &checkout))?;
        let handle = spawned.handle;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (spawned.stdin, spawned.stdout, spawned.stderr)
        else {
            end_child(&self.supervisor, handle, self.termination);
            return Err(ExternalAgentLaunchError::Spawn(
                "the child's stdio pipes were not all created".into(),
            ));
        };
        let stderr = self
            .supervisor
            .retain_stderr_tail_within(stderr, EXTERNAL_AGENT_STDERR_TAIL_BYTES);
        let (events, receiver) = mpsc::channel(EVENT_BUFFER);
        let pump = self.supervisor.spawn_pump(pump_stdout(stdout, events));
        let (input, lines) = mpsc::channel(INPUT_QUEUE);
        self.supervisor.spawn_pump(write_input(stdin, lines));
        Ok(Box::new(ClaudeCodeProcess {
            supervisor: Arc::clone(&self.supervisor),
            handle,
            input: std::sync::Mutex::new(Some(input)),
            events: Mutex::new(receiver),
            stderr,
            pump,
            termination: self.termination,
        }))
    }

    /// `claude` on the parent's `PATH`: the first absolute directory
    /// holding an executable regular file of that name.
    fn resolve_program(&self) -> Option<PathBuf> {
        let path = self
            .parent_environment
            .iter()
            .rev()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.as_os_str())?;
        resolve_on_path(path, CLAUDE_PROGRAM)
    }
}

impl ExternalAgentLauncher for ClaudeCodeLauncher {
    fn start<'a>(
        &'a self,
        spec: ExternalAgentLaunchSpec,
    ) -> PortFuture<'a, Result<Box<dyn ExternalAgentProcess>, ExternalAgentLaunchError>> {
        Box::pin(self.launch(spec))
    }
}

/// A refused credential is an invalid spec; only a directory the member's
/// state cannot live in is a member-directory failure.
fn environment_error(error: MemberEnvironmentError) -> ExternalAgentLaunchError {
    match error {
        MemberEnvironmentError::Credential(_) => invalid(error.to_string()),
        MemberEnvironmentError::Directory { .. } => {
            ExternalAgentLaunchError::MemberDirectory(error.to_string())
        }
    }
}

/// Red stub (#2286): a missing checkout still reads as a missing claude.
fn spawn_error(
    error: std::io::Error,
    program: &Path,
    _checkout: &Path,
) -> ExternalAgentLaunchError {
    match error.kind() {
        std::io::ErrorKind::NotFound => not_found(),
        _ => ExternalAgentLaunchError::Spawn(format!("{}: {error}", program.display())),
    }
}

fn not_found() -> ExternalAgentLaunchError {
    ExternalAgentLaunchError::NotFound {
        program: PROGRAM_NAME.into(),
        required_for: REQUIRED_FOR.into(),
    }
}

fn resolve_on_path(path: &OsStr, program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

fn checked_checkout(checkout: &Path) -> Result<PathBuf, ExternalAgentLaunchError> {
    if checkout.is_absolute() && checkout.is_dir() {
        return Ok(checkout.to_path_buf());
    }
    Err(invalid(format!(
        "the checkout {} is not an existing absolute directory",
        checkout.display()
    )))
}

fn invalid(detail: String) -> ExternalAgentLaunchError {
    ExternalAgentLaunchError::InvalidSpec(detail)
}

/// A model name: letters, digits and `-._[]`, not starting with `-`.
fn valid_model(model: &str) -> bool {
    model
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._[]".contains(c))
}

/// A tool name: letters, digits and `_`, starting with a letter.
fn valid_tool(tool: &str) -> bool {
    tool.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn json_object(flag: &str, value: &Value) -> Result<String, ExternalAgentLaunchError> {
    match value {
        Value::Object(_) => Ok(value.to_string()),
        _ => Err(invalid(format!("{flag} must be a JSON object"))),
    }
}

/// The argv after the program: the stream-json flags and the spec's values,
/// each value checked first.
pub fn claude_arguments(
    spec: &ExternalAgentLaunchSpec,
) -> Result<Vec<String>, ExternalAgentLaunchError> {
    if !valid_model(&spec.model) {
        return Err(invalid(format!(
            "model {:?} is not a model name",
            spec.model
        )));
    }
    if let Some(tool) = spec.tools.iter().find(|tool| !valid_tool(tool)) {
        return Err(invalid(format!("tool {tool:?} is not a tool name")));
    }
    if !(spec.max_budget_usd.is_finite() && spec.max_budget_usd > 0.0) {
        return Err(invalid(format!(
            "the budget {} is not a positive amount",
            spec.max_budget_usd
        )));
    }
    let mcp_config = json_object("--mcp-config", &spec.mcp_config)?;
    let settings = json_object("--settings", &spec.settings)?;
    let arguments: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        &spec.model,
        "--tools",
        &spec.tools.join(","),
        "--mcp-config",
        &mcp_config,
        "--strict-mcp-config",
        "--settings",
        &settings,
        "--setting-sources",
        "project",
        "--permission-mode",
        "bypassPermissions",
        "--no-session-persistence",
        "--max-budget-usd",
        &spec.max_budget_usd.to_string(),
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect();
    Ok(arguments)
}

/// Decode `stdout` line by line into `events` until it ends (or the
/// process handle is gone). Nothing a line holds ends the stream.
async fn pump_stdout(stdout: ChildStdout, events: mpsc::Sender<ExternalAgentEvent>) {
    let mut reader = BufReader::new(stdout);
    let mut decoder = StreamJsonDecoder::new();
    let mut line = Vec::new();
    loop {
        let read = match read_bounded_line_into(&mut reader, &mut line, STREAM_LINE_CAP_BYTES).await
        {
            Ok(Some(read)) => read,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, "claude stdout: read failed; the stream ends");
                return;
            }
        };
        if read.truncated {
            tracing::warn!(
                bytes = read.bytes_read,
                cap = STREAM_LINE_CAP_BYTES,
                "claude stream-json: line over the cap skipped"
            );
            continue;
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            tracing::warn!("claude stream-json: line that is not UTF-8 skipped");
            continue;
        };
        let text = text.trim_end_matches(['\n', '\r']);
        if text.trim().is_empty() {
            continue;
        }
        match decoder.decode_line(text) {
            Ok(decoded) => {
                for event in decoded {
                    if events.send(event).await.is_err() {
                        return;
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "claude stream-json: line skipped"),
        }
    }
}

/// One whole line for the stdin writer, and where it answers how the
/// write went.
struct InputLine {
    line: String,
    written: oneshot::Sender<Result<(), String>>,
}

/// Write each queued line whole, in order, until the queue closes (the
/// input was closed, or the process dropped) or a write fails; then close
/// stdin, which is `claude`'s shutdown request. A failed write ends the
/// writer: the lines behind it, and any later send, are answered as
/// failed, never written after a partial line.
async fn write_input(mut stdin: ChildStdin, mut lines: mpsc::Receiver<InputLine>) {
    while let Some(InputLine { line, written }) = lines.recv().await {
        debug_assert!(
            line.ends_with('\n') && line.matches('\n').count() == 1,
            "the writer is given exactly one whole line"
        );
        let outcome = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        }
        .await;
        match outcome {
            Ok(()) => {
                // A sender that stopped waiting still had its turn written.
                let _ = written.send(Ok(()));
            }
            Err(error) => {
                tracing::warn!(%error, "claude stdin: write failed; the input ends");
                let _ = written.send(Err(error.to_string()));
                break;
            }
        }
    }
    // EOF is the close itself; a failed shutdown of a pipe whose reader
    // already left changes nothing.
    let _ = stdin.shutdown().await;
}

/// One running `claude`. See the module docs.
pub struct ClaudeCodeProcess {
    supervisor: Arc<OwnedChildSupervisor>,
    handle: ChildHandleId,
    /// The stdin writer's queue; `None` once the input is closed. Held
    /// only to clone or take it, never across an await.
    input: std::sync::Mutex<Option<mpsc::Sender<InputLine>>>,
    events: Mutex<mpsc::Receiver<ExternalAgentEvent>>,
    stderr: StderrTail,
    pump: tokio::task::JoinHandle<()>,
    termination: TerminationBudget,
}

/// One user turn as the stream-json user message, one line.
fn user_message_line(text: &str) -> String {
    let message = json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
    });
    let mut line = message.to_string();
    debug_assert!(!line.contains('\n'), "a turn is exactly one line");
    line.push('\n');
    line
}

impl ClaudeCodeProcess {
    fn input(&self) -> std::sync::MutexGuard<'_, Option<mpsc::Sender<InputLine>>> {
        self.input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Queue `text` as one whole line and wait until it is written.
    /// Cancel-safe: dropped before it is queued, nothing is sent; dropped
    /// after, the line is still written whole.
    async fn write_turn(&self, text: &str) -> Result<(), ExternalAgentInputError> {
        let line = user_message_line(text);
        let queue = self.input().clone();
        let Some(queue) = queue else {
            return Err(ExternalAgentInputError::Closed);
        };
        let (written, answer) = oneshot::channel();
        let queued = queue.send(InputLine { line, written }).await;
        // The clone must not keep the queue open past a close.
        drop(queue);
        queued.map_err(|_| gone())?;
        match answer.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(ExternalAgentInputError::Write(error)),
            Err(_) => Err(gone()),
        }
    }
}

/// The writer ended (a write failed) before this turn was written.
fn gone() -> ExternalAgentInputError {
    ExternalAgentInputError::Write("claude's input ended after a failed write".into())
}

impl ExternalAgentProcess for ClaudeCodeProcess {
    fn send_user_turn<'a>(
        &'a self,
        text: &'a str,
    ) -> PortFuture<'a, Result<(), ExternalAgentInputError>> {
        Box::pin(self.write_turn(text))
    }

    fn next_event(&self) -> PortFuture<'_, Option<ExternalAgentEvent>> {
        Box::pin(async move { self.events.lock().await.recv().await })
    }

    fn close_input(&self) -> PortFuture<'_, ()> {
        // Closing the queue is the close: the writer closes stdin once the
        // turns already queued are written. Nothing here waits on a write.
        Box::pin(async move {
            let queue = self.input().take();
            drop(queue);
        })
    }

    fn exited(&self) -> PortFuture<'_, ExternalAgentExit> {
        Box::pin(async move {
            match self.supervisor.wait_exit(self.handle).await {
                Some(ChildExit::Code(code)) => ExternalAgentExit::Code(code),
                Some(ChildExit::Signal(signal)) => ExternalAgentExit::Signal(signal),
                Some(ChildExit::Unobservable(detail)) => ExternalAgentExit::Unobservable(detail),
                None => ExternalAgentExit::Unobservable(
                    "the supervisor no longer knows this child".into(),
                ),
            }
        })
    }

    fn stderr_tail(&self) -> String {
        self.stderr.snapshot()
    }
}

impl Drop for ClaudeCodeProcess {
    fn drop(&mut self) {
        self.pump.abort();
        // Closing the input is the shutdown request `claude` answers by
        // exiting: the writer closes stdin once its queue is drained.
        let queue = self
            .input
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        drop(queue);
        end_child(&self.supervisor, self.handle, self.termination);
    }
}

/// Have the supervisor end `handle`'s child once its input is closed: it
/// exits by itself, or the supervisor's TERM → KILL fallback ends its
/// group. Its slot is retired once it is reaped.
fn end_child(
    supervisor: &Arc<OwnedChildSupervisor>,
    handle: ChildHandleId,
    budget: TerminationBudget,
) {
    supervisor.request_termination(
        handle,
        Box::pin(async { ProtocolOutcome::Acknowledged }),
        budget,
    );
    supervisor.retire_when_reaped(handle);
}

#[cfg(test)]
#[path = "process_test_rig.rs"]
mod test_rig;

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "process_input_tests.rs"]
mod input_tests;

#[cfg(test)]
#[path = "process_stream_tests.rs"]
mod stream_tests;

#[cfg(test)]
#[path = "process_telemetry_tests.rs"]
mod telemetry_tests;
