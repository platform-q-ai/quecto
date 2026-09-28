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
//! message and decodes stdout through [`super::stream_json`] on a pump
//! task: a line that is not a JSON object (or not UTF-8, or longer than
//! [`STREAM_LINE_CAP_BYTES`]) is skipped and logged, never the end of the
//! stream. Stderr keeps its last [`EXTERNAL_AGENT_STDERR_TAIL_BYTES`].
//! Closing the input makes `claude` finish and exit 0. Dropping the process
//! closes its input and asks the supervisor to end the child — first by
//! that close, then TERM and KILL to its group if it does not exit.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use quecto_line_io::read_bounded_line_into;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{Mutex, mpsc};

use super::environment::member_environment;
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
const EVENT_BUFFER: usize = 256;

/// Starts `claude` member processes.
pub struct ClaudeCodeLauncher {
    supervisor: Arc<OwnedChildSupervisor>,
    parent_environment: Vec<(OsString, OsString)>,
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
        }
    }

    /// A launcher over this process's own environment.
    pub fn from_process_environment(supervisor: Arc<OwnedChildSupervisor>) -> Self {
        Self::new(supervisor, std::env::vars_os().collect())
    }

    async fn launch(
        &self,
        spec: ExternalAgentLaunchSpec,
    ) -> Result<Box<dyn ExternalAgentProcess>, ExternalAgentLaunchError> {
        let arguments = claude_arguments(&spec)?;
        let checkout = checked_checkout(&spec.checkout)?;
        let program = self.resolve_program().ok_or_else(not_found)?;
        let environment =
            member_environment(&self.parent_environment, &spec.member_dir, &spec.credential)
                .map_err(|error| ExternalAgentLaunchError::MemberDirectory(error.to_string()))?;
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
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => not_found(),
                _ => ExternalAgentLaunchError::Spawn(format!("{}: {error}", program.display())),
            })?;
        let handle = spawned.handle;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (spawned.stdin, spawned.stdout, spawned.stderr)
        else {
            end_child(&self.supervisor, handle);
            return Err(ExternalAgentLaunchError::Spawn(
                "the child's stdio pipes were not all created".into(),
            ));
        };
        let stderr = self
            .supervisor
            .retain_stderr_tail_within(stderr, EXTERNAL_AGENT_STDERR_TAIL_BYTES);
        let (events, receiver) = mpsc::channel(EVENT_BUFFER);
        let pump = tokio::spawn(pump_stdout(stdout, events));
        Ok(Box::new(ClaudeCodeProcess {
            supervisor: Arc::clone(&self.supervisor),
            handle,
            stdin: Mutex::new(Some(stdin)),
            events: Mutex::new(receiver),
            stderr,
            pump,
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

/// One running `claude`. See the module docs.
pub struct ClaudeCodeProcess {
    supervisor: Arc<OwnedChildSupervisor>,
    handle: ChildHandleId,
    stdin: Mutex<Option<ChildStdin>>,
    events: Mutex<mpsc::Receiver<ExternalAgentEvent>>,
    stderr: StderrTail,
    pump: tokio::task::JoinHandle<()>,
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
    async fn write_turn(&self, text: &str) -> Result<(), ExternalAgentInputError> {
        let line = user_message_line(text);
        let mut stdin = self.stdin.lock().await;
        let Some(pipe) = stdin.as_mut() else {
            return Err(ExternalAgentInputError::Closed);
        };
        let written = async {
            pipe.write_all(line.as_bytes()).await?;
            pipe.flush().await
        }
        .await;
        written.map_err(|error| ExternalAgentInputError::Write(error.to_string()))
    }
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
        Box::pin(async move {
            let pipe = self.stdin.lock().await.take();
            if let Some(mut pipe) = pipe {
                // EOF is the close itself; a failed shutdown of a pipe
                // whose reader already left changes nothing.
                let _ = pipe.shutdown().await;
            }
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
        // Closing stdin is the shutdown request `claude` answers by exiting.
        drop(self.stdin.get_mut().take());
        end_child(&self.supervisor, self.handle);
    }
}

/// Have the supervisor end `handle`'s child once its input is closed: it
/// exits by itself, or the supervisor's TERM → KILL fallback ends its
/// group. Its slot is retired once it is reaped.
fn end_child(supervisor: &Arc<OwnedChildSupervisor>, handle: ChildHandleId) {
    supervisor.request_termination(
        handle,
        Box::pin(async { ProtocolOutcome::Acknowledged }),
        TerminationBudget::DEFAULT,
    );
    supervisor.retire_when_reaped(handle);
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
