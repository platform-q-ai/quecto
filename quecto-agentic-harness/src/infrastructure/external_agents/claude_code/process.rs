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
//! are pumped by the supervisor's line pumps
//! ([`crate::infrastructure::processes::child_line_pipes`]) on its own
//! runtime, never the caller's: a runtime the caller drops cannot end the
//! stream or the input of a child that lives on. The pumps move bytes
//! only; each line is decoded here, on the reader's side, in
//! [`ClaudeCodeProcess::next_event`].
//!
//! - **Output.** A line longer than the line cap or not UTF-8 becomes an
//!   [`ExternalAgentEvent::LineSkipped`] (it may have been the turn's
//!   `result`); a line that is not a JSON object is skipped and logged.
//!   Neither ends the stream. Lines wait for the reader within a byte
//!   budget ([`STREAM_BUFFER_BYTES`]); a reader that stops holds the child.
//! - **Input.** Each turn is one whole line, queued whole or not at all,
//!   so cancelling a send never leaves half a line on `claude`'s stdin.
//!   Closing the input answers every turn not yet taken by the writer
//!   [`ExternalAgentInputError::Closed`] and writes none of them; the one
//!   line already being written at the close is written whole. The close
//!   never waits behind a write stuck on a full pipe.
//! - Stderr keeps its last [`EXTERNAL_AGENT_STDERR_TAIL_BYTES`].
//! - **Telemetry.** Structured `tracing` events under
//!   [`TELEMETRY_TARGET`]: launch, process start, input close, skipped
//!   line, termination and exit — names, lengths and counts only, never a
//!   credential, a proxy value, an inline JSON argument or stderr itself.
//!
//! Closing the input makes `claude` finish and exit 0. Dropping the process
//! closes its input and asks the supervisor to end the child — first by
//! that close, then TERM and KILL to its group if it does not exit within
//! the launcher's [`TerminationBudget`].

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::environment::{MemberEnvironmentError, check_credential, member_environment};
use super::stream_json::StreamJsonDecoder;
use crate::application::external_agent::dto::{
    EXTERNAL_AGENT_STDERR_TAIL_BYTES, ExternalAgentExit, ExternalAgentInputError,
    ExternalAgentLaunchError, ExternalAgentLaunchSpec,
};
use crate::application::external_agent::ports::{
    ExternalAgentLauncher, ExternalAgentProcess, PortFuture,
};
use crate::domain::external_agent::stream::{ExternalAgentEvent, SkippedLine, SkippedLineReason};
use crate::domain::redaction::redact_secrets;
use crate::infrastructure::processes::child_line_pipes::{
    LineLimits, LineWriteError, StdinLines, StdoutLine, StdoutLines,
};
use crate::infrastructure::processes::child_stderr_tail::StderrTail;
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildExit, ChildHandleId, OwnedChildSupervisor, ProcessGroup, ProtocolOutcome,
    TerminationBudget, TerminationOutcome,
};

/// The program looked up on `PATH`.
pub const CLAUDE_PROGRAM: &str = "claude";

/// How the missing program is named in the launch error.
const PROGRAM_NAME: &str = "claude CLI";

/// What the program is required for, in the launch error.
const REQUIRED_FOR: &str = "claude-code members";

/// The `tracing` target of a member process's telemetry.
pub const TELEMETRY_TARGET: &str = "quecto::external_agent";

/// The longest stdout line decoded; a longer one is skipped as a
/// [`ExternalAgentEvent::LineSkipped`].
pub const STREAM_LINE_CAP_BYTES: usize = 16 * 1024 * 1024;

/// The bytes of stdout lines buffered ahead of the reader; a full budget
/// holds the pump (and so the child's stdout) until the session reads on.
/// Worst case held for one member: this budget plus the one line the pump
/// is reading (at most [`STREAM_LINE_CAP_BYTES`]), 48 MiB. Real
/// stream-json lines are KiB.
pub const STREAM_BUFFER_BYTES: usize = 32 * 1024 * 1024;

/// How long an exit waits for stderr's end, so the tail carries the
/// child's last words; a descendant holding stderr open cannot hold the
/// exit longer.
pub const STDERR_EOF_GRACE: Duration = Duration::from_millis(500);

/// Whole turns queued ahead of the stdin writer; a full queue holds the
/// sender (cancel-safely: an unqueued turn is simply not sent).
const INPUT_QUEUE: usize = 16;

/// Every flag `claude` is given: the only arguments telemetry names (a
/// flag's value may be inline JSON).
const CLAUDE_FLAGS: &[&str] = &[
    "-p",
    "--input-format",
    "--output-format",
    "--verbose",
    "--model",
    "--tools",
    "--mcp-config",
    "--strict-mcp-config",
    "--settings",
    "--setting-sources",
    "--permission-mode",
    "--allow-dangerously-skip-permissions",
    "--no-session-persistence",
    "--max-budget-usd",
];

/// Starts `claude` member processes.
pub struct ClaudeCodeLauncher {
    supervisor: Arc<OwnedChildSupervisor>,
    parent_environment: Vec<(OsString, OsString)>,
    termination: TerminationBudget,
    limits: LineLimits,
    stderr_eof_grace: Duration,
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
            stderr_eof_grace: STDERR_EOF_GRACE,
            limits: LineLimits {
                line_cap: STREAM_LINE_CAP_BYTES,
                buffer_bytes: STREAM_BUFFER_BYTES,
            },
        }
    }

    /// This launcher, ending a dropped process's child within `budget`
    /// instead of [`TerminationBudget::DEFAULT`].
    pub fn with_termination_budget(mut self, budget: TerminationBudget) -> Self {
        self.termination = budget;
        self
    }

    /// This launcher, reading lines of at most `line_cap` bytes with at
    /// most `buffer_bytes` of them waiting for the reader, instead of
    /// [`STREAM_LINE_CAP_BYTES`] and [`STREAM_BUFFER_BYTES`]. A line must
    /// fit the buffer.
    pub fn with_stream_limits(mut self, line_cap: usize, buffer_bytes: usize) -> Self {
        let limits = LineLimits {
            line_cap,
            buffer_bytes,
        };
        assert!(limits.valid(), "invalid stream limits: {limits:?}");
        self.limits = limits;
        self
    }

    /// This launcher, waiting at most `grace` for stderr's end before an
    /// exit is returned, instead of [`STDERR_EOF_GRACE`].
    pub fn with_stderr_eof_grace(mut self, grace: Duration) -> Self {
        self.stderr_eof_grace = grace;
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
        let started = Instant::now();
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
        let member = member_name(&spec.member_dir);
        tracing::info!(
            target: TELEMETRY_TARGET,
            member = %member,
            flags = ?flags_of(&arguments),
            env = ?environment.names(),
            dirs = ?[&environment.home, &environment.config_dir],
            "claude member launch"
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
        // `ProcessGroup::Own`: the supervisor made the child lead its own
        // group, so its pgid is its pid.
        tracing::info!(
            target: TELEMETRY_TARGET,
            member = %member,
            pid = spawned.display_pid.0,
            pgid = spawned.display_pid.0,
            "claude member process started"
        );
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (spawned.stdin, spawned.stdout, spawned.stderr)
        else {
            end_child(&self.supervisor, handle, self.termination, member);
            return Err(ExternalAgentLaunchError::Spawn(
                "the child's stdio pipes were not all created".into(),
            ));
        };
        let stderr = self
            .supervisor
            .retain_stderr_tail_within(stderr, EXTERNAL_AGENT_STDERR_TAIL_BYTES);
        let lines = self.supervisor.pump_stdout_lines(stdout, self.limits);
        let input = self.supervisor.pump_stdin_lines(stdin, INPUT_QUEUE);
        Ok(Box::new(ClaudeCodeProcess {
            supervisor: Arc::clone(&self.supervisor),
            handle,
            member,
            started,
            input,
            events: Mutex::new(EventStream {
                lines,
                decoder: StreamJsonDecoder::new(),
                pending: VecDeque::new(),
            }),
            discarded_lines: AtomicUsize::new(0),
            exit_logged: AtomicBool::new(false),
            stderr,
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

/// The member's name in telemetry: its member directory's last component.
fn member_name(member_dir: &Path) -> String {
    member_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The flags of `arguments`, without their values.
fn flags_of(arguments: &[String]) -> Vec<&str> {
    arguments
        .iter()
        .map(String::as_str)
        .filter(|argument| CLAUDE_FLAGS.contains(argument))
        .collect()
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

/// A spawn's `NotFound` is either the working directory or the program:
/// the checkout is checked again (it was checked before the member's
/// directories were made, and may have gone since), so a checkout removed
/// under the launch is never reported as a missing `claude`.
fn spawn_error(error: std::io::Error, program: &Path, checkout: &Path) -> ExternalAgentLaunchError {
    match error.kind() {
        std::io::ErrorKind::NotFound => match checked_checkout(checkout) {
            Ok(_) => not_found(),
            Err(checkout_gone) => checkout_gone,
        },
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
///
/// `--mcp-config` and `--settings` are inline JSON, and argv is readable
/// by every local user through procfs. Nothing secret may be inlined: the
/// credential reaches `claude` only through its environment, and S5 must
/// hand bridge tokens over by a file in the member's private directory,
/// never in this JSON. An argv that would carry the credential's value is
/// refused.
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
        // Spike #2264's shim passed this whenever the mode was
        // bypassPermissions. Claude 2.1.280 honoured the mode without it
        // (round-1 live check), but a version that requires the opt-in
        // would silently fall back to prompting: match the spike.
        "--allow-dangerously-skip-permissions",
        "--no-session-persistence",
        "--max-budget-usd",
        &spec.max_budget_usd.to_string(),
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect();
    debug_assert!(
        arguments
            .iter()
            .filter(|argument| argument.starts_with('-'))
            .all(|flag| CLAUDE_FLAGS.contains(&flag.as_str())),
        "every flag is one telemetry may name"
    );
    let secret = spec.credential.value.as_str();
    let argv_clean =
        secret.is_empty() || arguments.iter().all(|argument| !argument.contains(secret));
    if argv_clean {
        return Ok(arguments);
    }
    Err(invalid(format!(
        "the argv would carry the value of {}: argv is readable by every local user",
        spec.credential.name
    )))
}

/// Stdout's lines and what they decoded to, read under one lock.
struct EventStream {
    lines: StdoutLines,
    decoder: StreamJsonDecoder,
    /// Events of a line already decoded, not yet returned.
    pending: VecDeque<ExternalAgentEvent>,
}

impl EventStream {
    /// The next event; `None` once stdout has ended. Cancel-safe: the only
    /// await is the line pump's, and a line is decoded whole before the
    /// next await.
    async fn next(&mut self, member: &str) -> Option<ExternalAgentEvent> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            match self.lines.next().await? {
                StdoutLine::OverCap { bytes } => {
                    return Some(skipped(member, SkippedLineReason::OverCap, bytes));
                }
                StdoutLine::Line(bytes) => {
                    if let Some(event) = self.decode(member, &bytes) {
                        return Some(event);
                    }
                }
            }
        }
    }

    /// Decode one line into [`Self::pending`]; a line that is not UTF-8 is
    /// returned as a skipped line instead.
    fn decode(&mut self, member: &str, bytes: &[u8]) -> Option<ExternalAgentEvent> {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return Some(skipped(member, SkippedLineReason::NotUtf8, bytes.len()));
        };
        let text = text.trim_end_matches(['\n', '\r']);
        let blank = text.trim().is_empty();
        if blank {
            return None;
        }
        match self.decoder.decode_line(text) {
            Ok(decoded) => self.pending.extend(decoded),
            Err(error) => tracing::warn!(
                target: TELEMETRY_TARGET,
                member = %member,
                bytes = bytes.len(),
                error = %redact_secrets(&error.to_string()),
                "claude member line not decoded"
            ),
        }
        None
    }
}

/// A skipped line, logged.
fn skipped(member: &str, reason: SkippedLineReason, bytes: usize) -> ExternalAgentEvent {
    tracing::warn!(
        target: TELEMETRY_TARGET,
        member = %member,
        reason = ?reason,
        bytes,
        "claude member line skipped"
    );
    ExternalAgentEvent::LineSkipped(SkippedLine { reason, bytes })
}

/// One running `claude`. See the module docs.
pub struct ClaudeCodeProcess {
    supervisor: Arc<OwnedChildSupervisor>,
    handle: ChildHandleId,
    /// The member's name, for telemetry.
    member: String,
    started: Instant,
    input: StdinLines,
    events: Mutex<EventStream>,
    /// Lines [`ExternalAgentProcess::exited`] discarded unread.
    discarded_lines: AtomicUsize,
    exit_logged: AtomicBool,
    stderr: StderrTail,
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
    /// Close the input, logging the first close and what caused it.
    fn close(&self, cause: &'static str) {
        if self.input.close() {
            tracing::info!(
                target: TELEMETRY_TARGET,
                member = %self.member,
                cause,
                "claude member stdin closed"
            );
        }
    }

    /// Read and discard stdout until it ends, so a child whose output no
    /// one reads can still finish writing and exit; then wait forever.
    async fn drain_unread(&self) -> std::convert::Infallible {
        let mut events = self.events.lock().await;
        events.pending.clear();
        while events.lines.next().await.is_some() {
            self.discarded_lines.fetch_add(1, Ordering::Relaxed);
        }
        std::future::pending().await
    }

    fn log_exit(&self, exit: &ExternalAgentExit) {
        if self.exit_logged.swap(true, Ordering::Relaxed) {
            return;
        }
        let (status, signal, unobservable) = match exit {
            ExternalAgentExit::Code(code) => (Some(*code), None, None),
            ExternalAgentExit::Signal(signal) => (None, Some(*signal), None),
            ExternalAgentExit::Unobservable(detail) => (None, None, Some(redact_secrets(detail))),
        };
        tracing::info!(
            target: TELEMETRY_TARGET,
            member = %self.member,
            status,
            signal,
            unobservable,
            wall_ms = self.started.elapsed().as_millis() as u64,
            stderr_tail_bytes = self.stderr.snapshot().len(),
            discarded_lines = self.discarded_lines.load(Ordering::Relaxed),
            "claude member exit"
        );
    }
}

impl ExternalAgentProcess for ClaudeCodeProcess {
    fn send_user_turn<'a>(
        &'a self,
        text: &'a str,
    ) -> PortFuture<'a, Result<(), ExternalAgentInputError>> {
        Box::pin(async move {
            self.input
                .write_line(user_message_line(text))
                .await
                .map_err(|error| match error {
                    LineWriteError::Closed => ExternalAgentInputError::Closed,
                    LineWriteError::Failed(detail) => ExternalAgentInputError::Write(detail),
                })
        })
    }

    fn next_event(&self) -> PortFuture<'_, Option<ExternalAgentEvent>> {
        Box::pin(async move { self.events.lock().await.next(&self.member).await })
    }

    fn close_input(&self) -> PortFuture<'_, ()> {
        // The writer closes stdin once no taken line is left; nothing here
        // waits on a write.
        Box::pin(async move { self.close("close_input") })
    }

    /// Waits for the exit while draining stdout: events not read by now
    /// are discarded (counted in the exit's telemetry), so a caller that
    /// stopped reading cannot hold the child, and this wait, forever.
    fn exited(&self) -> PortFuture<'_, ExternalAgentExit> {
        Box::pin(async move {
            let exit = tokio::select! {
                biased;
                exit = self.supervisor.wait_exit(self.handle) => exit,
                never = self.drain_unread() => match never {},
            };
            let exit = match exit {
                Some(ChildExit::Code(code)) => ExternalAgentExit::Code(code),
                Some(ChildExit::Signal(signal)) => ExternalAgentExit::Signal(signal),
                Some(ChildExit::Unobservable(detail)) => ExternalAgentExit::Unobservable(detail),
                None => ExternalAgentExit::Unobservable(
                    "the supervisor no longer knows this child".into(),
                ),
            };
            self.log_exit(&exit);
            exit
        })
    }

    fn exited_discarding_output(&self) -> PortFuture<'_, ExternalAgentExit> {
        self.exited()
    }

    fn stderr_tail(&self) -> String {
        self.stderr.snapshot()
    }
}

impl Drop for ClaudeCodeProcess {
    fn drop(&mut self) {
        // Closing the input is the shutdown request `claude` answers by
        // exiting. The stdout pump stops with its `StdoutLines`.
        self.close("drop");
        end_child(
            &self.supervisor,
            self.handle,
            self.termination,
            std::mem::take(&mut self.member),
        );
    }
}

/// The last signal a termination needed.
fn signal_of(outcome: &TerminationOutcome) -> &'static str {
    match outcome {
        TerminationOutcome::ExitedAfterKill { .. } | TerminationOutcome::StillRunning { .. } => {
            "KILL"
        }
        TerminationOutcome::ExitedAfterTerm { .. } => "TERM",
        TerminationOutcome::NoRetainedHandle
        | TerminationOutcome::AlreadyExited(_)
        | TerminationOutcome::ExitedAfterProtocol(_) => "none",
    }
}

/// Have the supervisor end `handle`'s child once its input is closed: it
/// exits by itself, or the supervisor's TERM → KILL fallback ends its
/// group. Its slot is retired once it is reaped. Members of its group left
/// running after the leader is reaped (a grandchild that ignored TERM
/// before a KILL that never came) are the S8 sweep's (#2292).
fn end_child(
    supervisor: &Arc<OwnedChildSupervisor>,
    handle: ChildHandleId,
    budget: TerminationBudget,
    member: String,
) {
    supervisor.request_termination_observed(
        handle,
        Box::pin(async { ProtocolOutcome::Acknowledged }),
        budget,
        Box::new(move |outcome, elapsed| {
            tracing::info!(
                target: TELEMETRY_TARGET,
                member = %member,
                signal = %signal_of(outcome),
                still_running = matches!(outcome, TerminationOutcome::StillRunning { .. }),
                elapsed_ms = elapsed.as_millis() as u64,
                "claude member termination"
            );
        }),
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
