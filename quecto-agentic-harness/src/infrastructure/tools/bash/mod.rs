// Shell execution tool: impl Tool for ExecTool (bash).
//
// Commands run natively under bash where it is installed (see `shell`, #2195)
// in the configured workspace.
// Isolation is delegated to the deployment (e.g. running Quecto in a
// container); the in-process command policy still blocks configured dangerous
// commands before execution.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

/// Default per-command capture cap (10 MiB). Output beyond this is truncated.
const MAX_CAPTURE_BYTES: usize = 10 * 1024 * 1024;
/// Grace window for draining stdout/stderr after a timed-out child is killed.
const STREAM_DRAIN_TIMEOUT_ON_KILL: Duration = Duration::from_millis(250);
/// After the shell exits, the call waits for its output to end — until no
/// output has arrived for this long (#2167): only a background job still
/// holds the pipes then, and the call does not wait for it.
const QUIET_AFTER_EXIT: Duration = Duration::from_millis(500);
/// The longest the call waits after the shell exits, for a background job
/// that keeps writing.
const CEILING_AFTER_EXIT: Duration = Duration::from_secs(5);
/// Said when a background job still held the output after the shell exited.
const BACKGROUND_NOTE: &str = "[a background process still holds this command's output; the call \
     returned without waiting for it, and its later output is discarded. Redirect it to keep it, \
     e.g. `cmd > out.log 2>&1 &`]";

mod binary;
mod capture;
mod inline_output;
mod long_lines;
mod saved_output;
#[cfg(test)]
use capture::read_stream_limited;
use capture::{Stream, StreamReader};
use inline_output::truncate_output;
#[cfg(test)]
use saved_output::save_to_temp_file;

#[derive(Debug, Clone)]
pub struct ExecOptions {
    pub timeout: Duration,
    pub max_capture_bytes: usize,
    /// Optional string prepended to every command before execution.
    /// Useful for setting environment variables or aliases (e.g. `shopt -s expand_aliases`).
    /// Separated from the actual command by `\n`.
    pub command_prefix: Option<String>,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            // No default timeout — processes run indefinitely unless configured.
            timeout: Duration::MAX,
            max_capture_bytes: MAX_CAPTURE_BYTES,
            command_prefix: None,
        }
    }
}

pub struct ExecTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    timeout: Duration,
    max_capture_bytes: usize,
    /// Optional string prepended to every command (e.g. alias setup or env exports).
    command_prefix: Option<String>,
}

impl std::fmt::Debug for ExecTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecTool")
            .field("workspace", &self.workspace)
            .field("timeout", &self.timeout)
            .field("max_capture_bytes", &self.max_capture_bytes)
            .finish()
    }
}

impl ExecTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self::with_options(workspace, sandbox, ExecOptions::default())
    }

    pub fn with_timeout(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>, timeout: Duration) -> Self {
        let opts = ExecOptions {
            timeout,
            ..ExecOptions::default()
        };
        Self::with_options(workspace, sandbox, opts)
    }

    pub fn with_options(
        workspace: Arc<PathBuf>,
        sandbox: Arc<Sandbox>,
        options: ExecOptions,
    ) -> Self {
        Self {
            workspace,
            sandbox,
            timeout: options.timeout,
            max_capture_bytes: options.max_capture_bytes,
            command_prefix: options.command_prefix,
        }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn execute_with_env(
        &self,
        arguments: &str,
        env_overrides: &HashMap<String, String>,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        let env_overrides = env_overrides.clone();

        Box::pin(async move { self.run_command(&args_str, Some(&env_overrides)).await })
    }

    async fn run_command(
        &self,
        arguments: &str,
        env_overrides: Option<&HashMap<String, String>>,
    ) -> Result<ToolResult, DomainError> {
        // LLM-addressable: malformed JSON → Ok(is_error=true). Tool contract.
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!(
                        "invalid JSON arguments: {e}. Example: {{\"command\": \"ls -la\"}}"
                    ),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                });
            }
        };
        let Some(command) = args["command"].as_str().map(str::to_string) else {
            return Ok(ToolResult {
                content: "missing 'command' argument. Example: {\"command\": \"ls -la\"}"
                    .to_string(),
                is_error: true,
                image_blocks: vec![],
                delivery_metadata: None,
            });
        };
        let per_invocation_timeout = parse_timeout(&args);
        let output_file = args["output_file"].as_str().map(str::to_string);

        // Per-invocation timeout is capped at the configured maximum.
        let effective_timeout = match per_invocation_timeout {
            Some(requested) => requested.min(self.timeout),
            None => self.timeout,
        };

        // Apply command prefix if configured (prefix is a trusted construction-time option).
        // Security: validate the user-supplied command first, then build the full command.
        // The prefix runs before the validated command; it must be trusted by the deployer.
        self.sandbox
            .validate_command(&command)
            .map_err(|e| DomainError::Security(e.to_string()))?;

        let full_command = match &self.command_prefix {
            Some(prefix) => format!("{}\n{}", prefix, command),
            None => command,
        };

        self.spawn_and_wait(&full_command, env_overrides, effective_timeout, output_file)
            .await
    }

    async fn spawn_and_wait(
        &self,
        command: &str,
        source_env: Option<&HashMap<String, String>>,
        timeout_dur: Duration,
        output_file: Option<String>,
    ) -> Result<ToolResult, DomainError> {
        let mut cmd = build_shell_command(&self.workspace, command, source_env);

        let output_target = match output_file.as_deref() {
            Some(path) => Some(prepare_output_file(&self.workspace, path).await?),
            None => None,
        };
        if let Some(target) = &output_target {
            let stdout = target
                .file
                .try_clone()
                .map_err(|e| DomainError::Tool(format!("bash output_file failed: {}", e)))?;
            let stderr = target
                .file
                .try_clone()
                .map_err(|e| DomainError::Tool(format!("bash output_file failed: {}", e)))?;
            cmd.stdout(std::process::Stdio::from(stdout))
                .stderr(std::process::Stdio::from(stderr));
        } else {
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| DomainError::Tool(format!("bash failed: {}", e)))?;

        let stdout_task = child
            .stdout
            .take()
            .map(|pipe| StreamReader::spawn(pipe, self.max_capture_bytes, Stream::Stdout));
        let stderr_task = child
            .stderr
            .take()
            .map(|pipe| StreamReader::spawn(pipe, self.max_capture_bytes, Stream::Stderr));

        let stream_tasks = StreamTasks {
            stdout_task,
            stderr_task,
        };

        run_child_with_timeout(
            child,
            stream_tasks,
            timeout_dur,
            self.workspace.clone(),
            output_target,
        )
        .await
    }
}

/// Best-effort reaper that kills a child's process group on drop (#895 AC3).
///
/// Spawned children run in their own group (`process_group(0)`), so signalling
/// the negative pgid terminates the shell AND any descendants it forked. Signals
/// directly via `libc::kill` rather than `kill(1)`, whose negative-pgid handling
/// is shell-dependent (dash's builtin silently no-ops on it).
struct ProcessGroupGuard {
    pid: Option<u32>,
}

impl ProcessGroupGuard {
    fn new(pid: Option<u32>) -> Self {
        Self { pid }
    }

    /// Cancel the on-drop kill — the child has already been awaited/reaped.
    fn disarm(&mut self) {
        self.pid = None;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            // SIGKILL the child's whole process group. A negative pid signals the
            // entire group (pgid == child pid, set via `process_group(0)`), so the
            // leader AND any descendants it forked are reaped. We signal directly
            // via `libc::kill` rather than shelling out to `kill(1)`: the negative
            // -pgid form is parsed inconsistently across shells/`kill`
            // implementations (e.g. dash's builtin silently no-ops on it, so the
            // group was never signalled on Debian/Ubuntu `/bin/sh` hosts). The
            // syscall is unambiguous and near-instant.
            //
            // `kill(2)` is async-signal-safe and takes plain integers; we pass a
            // pid we own and a constant signal, so it cannot violate Rust memory
            // safety. The leader is retained unreaped while this guard is armed;
            // ESRCH is not used as an identity check.
            // SAFETY: FFI call to `libc::kill` with owned-pid + constant signal.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
    }
}

/// Parse optional per-invocation timeout from a JSON args value.
///
/// Returns `Some(timeout)` when a `timeout` key is present and positive,
/// or `None` otherwise. Callers cap the returned timeout at the configured
/// maximum.
fn parse_timeout(args: &serde_json::Value) -> Option<Duration> {
    // Accept both integer and float timeout values (schema says "number").
    // as_u64() returns None for floats; use as_f64() and round for broad compatibility.
    args["timeout"].as_f64().and_then(|f| {
        let secs = f.round() as u64;
        if secs > 0 {
            Some(Duration::from_secs(secs))
        } else {
            None // timeout=0 → use default
        }
    })
}

fn build_shell_command(
    workspace: &PathBuf,
    command: &str,
    source_env: Option<&HashMap<String, String>>,
) -> tokio::process::Command {
    // Bash where installed (#2195): the process's choice, made once, unless
    // an explicit environment names its own SHELL.
    let shell = match source_env {
        Some(env) => shell::select_shell(env.get("SHELL").map(String::as_str), shell::is_installed),
        None => shell::default_shell(),
    };

    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-c").arg(command).current_dir(workspace);

    // Put the child in its own process group (pgid == child pid) so a cancel can
    // kill the WHOLE tree — the shell plus any grandchildren — not just the shell
    // leader (#895 AC3). `kill_on_drop` then guarantees the leader dies when the
    // in-flight tool future is dropped on abort; the group kill in
    // `ProcessGroupGuard` reaps the rest.
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.kill_on_drop(true);

    if let Some(source_env) = source_env {
        cmd.env_clear();
        for (k, v) in source_env {
            cmd.env(k, v);
        }

        if !source_env.contains_key("PATH") {
            if let Ok(path) = std::env::var("PATH") {
                cmd.env("PATH", path);
            }
        }
    }

    // Ordinary commands (including cargo-spawned test runtimes) are not
    // admitted swarm members. Only the managed spawn adapter grants identity.
    // RUST_LOG is the harness's own log level (a container adapter sets it so
    // member logs reach journald, #1924); a member's `cargo test` or any
    // other Rust tool child must not inherit it.
    for key in [
        "QUECTO_SWARM_CHECKOUT",
        "QUECTO_SWARM_MEMBER",
        "QUECTO_SWARM_RESERVATION",
        "QUECTO_SWARM_BOOTSTRAP",
        "QUECTO_SWARM_CONTAINER",
        "QUECTO_SWARM_HOST_PID_NS",
        // The harness's watermark marks (#2403): a command decides its own.
        "QUECTO_CONTEXT_HIGH_TOKENS",
        "QUECTO_CONTEXT_LOW_TOKENS",
        "RUST_LOG",
        // A non-interactive shell sources these before the command: code
        // the command policy never saw (#2207 review).
        "BASH_ENV",
        "ENV",
    ] {
        cmd.env_remove(key);
    }
    cmd
}

struct StreamTasks {
    stdout_task: Option<StreamReader>,
    stderr_task: Option<StreamReader>,
}

mod wait_owned;

async fn run_child_with_timeout(
    mut child: tokio::process::Child,
    mut stream_tasks: StreamTasks,
    timeout_dur: Duration,
    workspace: Arc<PathBuf>,
    output_target: Option<OutputTarget>,
) -> Result<ToolResult, DomainError> {
    // Declared after Child so cancellation signals before Child drops/reaps.
    let mut group_guard = ProcessGroupGuard::new(child.id());
    // On Unix, retain the unreaped leader (and thus PGID ownership) through
    // pipe drainage: same-group survivors may still need cancellation cleanup.
    #[cfg(unix)]
    let completion = tokio::time::timeout(timeout_dur, wait_owned::exited(&mut child)).await;
    #[cfg(not(unix))]
    let completion = tokio::time::timeout(timeout_dur, child.wait()).await;
    match completion {
        Ok(Ok(observed)) => {
            #[cfg(not(unix))]
            group_guard.disarm();
            let (output, capture_cut, held_open) = collect_after_exit(&mut stream_tasks).await;
            group_guard.disarm();
            #[cfg(unix)]
            let status = {
                let () = observed;
                child
                    .wait()
                    .await
                    .map_err(|e| DomainError::Tool(format!("bash failed: {e}")))?
            };
            #[cfg(not(unix))]
            let status = observed;
            let mut content = match output_target {
                Some(target) => output_file_summary(&target, None),
                None => truncate_output(output, capture_cut).await,
            };
            if held_open {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(BACKGROUND_NOTE);
            }
            Ok(make_exit_result(status, content))
        }
        Ok(Err(e)) => {
            group_guard.disarm();
            Err(DomainError::Tool(format!("bash failed: {}", e)))
        }
        Err(_) => {
            // handle_timeout signals while Child is still owned, then disarms
            // before reaping/draining. The outer frame must not retain a PGID.
            group_guard.disarm();
            Ok(handle_timeout(child, stream_tasks, timeout_dur, workspace, output_target).await)
        }
    }
}

/// Collect stdout + stderr, truncate, and append the note on what is shown
/// (see [`inline_output::truncate_output`]).
#[cfg(test)]
async fn collect_and_truncate_output(stream_tasks: &mut StreamTasks) -> String {
    let (output, capture_cut, _) = collect_after_exit(stream_tasks).await;
    truncate_output(output, capture_cut).await
}

/// The output once the shell has exited: each stream to its end, or what
/// arrived before it went quiet when a background job still holds it
/// (#2167). Returns the output, whether a capture dropped part of it, and
/// whether a stream was still held open.
async fn collect_after_exit(stream_tasks: &mut StreamTasks) -> (String, bool, bool) {
    let (stdout, stderr) = tokio::join!(
        await_stream_output_within(
            stream_tasks.stdout_task.take(),
            QUIET_AFTER_EXIT,
            CEILING_AFTER_EXIT
        ),
        await_stream_output_within(
            stream_tasks.stderr_task.take(),
            QUIET_AFTER_EXIT,
            CEILING_AFTER_EXIT
        )
    );
    let ((stdout, stdout_cut), stdout_open) = stdout;
    let ((stderr, stderr_cut), stderr_open) = stderr;
    (
        combine(stdout, stderr),
        stdout_cut || stderr_cut,
        stdout_open || stderr_open,
    )
}

fn combine(stdout: String, stderr: String) -> String {
    if stderr.is_empty() {
        stdout
    } else if stdout.is_empty() {
        stderr
    } else {
        format!("{}\n{}", stdout, stderr)
    }
}

struct OutputTarget {
    path: PathBuf,
    file: std::fs::File,
}

async fn prepare_output_file(
    workspace: &std::path::Path,
    path: &str,
) -> Result<OutputTarget, DomainError> {
    let target = {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            workspace.join(p)
        }
    };
    let file = crate::infrastructure::tools::call_work::spawn_blocking_in_call({
        let target = target.clone();
        move || -> std::io::Result<std::fs::File> {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::File::create(&target)
        }
    })
    .await
    .map_err(|e| DomainError::Tool(format!("bash output_file failed: {}", e)))?
    .map_err(|e| DomainError::Tool(format!("bash output_file failed: {}", e)))?;
    Ok(OutputTarget { path: target, file })
}

fn output_file_summary(target: &OutputTarget, qualifier: Option<&str>) -> String {
    let (bytes, lines) = match std::fs::read_to_string(&target.path) {
        Ok(content) => (content.len(), content.lines().count()),
        Err(_) => (
            target.file.metadata().map(|m| m.len()).unwrap_or(0) as usize,
            0,
        ),
    };
    let qualifier = qualifier.unwrap_or("");
    format!(
        "output saved to: {}{}\nbytes: {}\nlines: {}",
        target.path.display(),
        qualifier,
        bytes,
        lines
    )
}

/// Build a ToolResult from a process exit status.
fn make_exit_result(status: std::process::ExitStatus, content: String) -> ToolResult {
    if status.success() {
        ToolResult {
            content,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        }
    } else {
        ToolResult {
            content: format!("exit code {}\n{}", status.code().unwrap_or(-1), content),
            is_error: true,
            image_blocks: vec![],
            delivery_metadata: None,
        }
    }
}

/// Kill the process and drain streams after a timeout.
async fn handle_timeout(
    mut child: tokio::process::Child,
    mut stream_tasks: StreamTasks,
    timeout_dur: Duration,
    _workspace: Arc<PathBuf>,
    output_target: Option<OutputTarget>,
) -> ToolResult {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // Timeout must reap the whole process group so grandchildren holding
        // stdout/stderr open do not prevent draining captured output.
        // SAFETY: FFI call to `libc::kill` with owned-pid + constant signal; stale/dead pids yield ESRCH.
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = child.kill().await;
    let _ = child.wait().await;
    // What arrived before the kill is kept even when a descendant outside
    // the group still holds a stream open (#2167).
    let ((stdout_raw, stdout_cut), _) = await_stream_output_within(
        stream_tasks.stdout_task.take(),
        STREAM_DRAIN_TIMEOUT_ON_KILL,
        STREAM_DRAIN_TIMEOUT_ON_KILL,
    )
    .await;
    let ((stderr_raw, stderr_cut), _) = await_stream_output_within(
        stream_tasks.stderr_task.take(),
        STREAM_DRAIN_TIMEOUT_ON_KILL,
        STREAM_DRAIN_TIMEOUT_ON_KILL,
    )
    .await;
    let capture_cut = stdout_cut || stderr_cut;
    let combined = combine(stdout_raw, stderr_raw);
    let file_note = if let Some(target) = &output_target {
        format!(
            "\n{}",
            output_file_summary(
                target,
                Some(" (captured before timeout; may be incomplete)")
            )
            .lines()
            .next()
            .unwrap_or("")
        )
    } else {
        String::new()
    };
    let tail = truncate_output(combined, capture_cut).await;
    let content = if tail.is_empty() {
        format!(
            "command timed out after {}s{}",
            timeout_dur.as_secs(),
            file_note
        )
    } else {
        format!(
            "command timed out after {}s{}\n{}",
            timeout_dur.as_secs(),
            file_note,
            tail
        )
    };
    ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

/// A stream to its end, or what arrived before it went quiet for `quiet` or
/// `ceiling` passed; the flag says the stream was still open.
async fn await_stream_output_within(
    task: Option<StreamReader>,
    quiet: Duration,
    ceiling: Duration,
) -> ((String, bool), bool) {
    match task {
        Some(reader) => reader.finish_within(quiet, ceiling).await,
        None => ((String::new(), false), false),
    }
}

/// Said in the description when no bash is installed (#2195): bash syntax
/// will not work there.
fn posix_note(shell: &str) -> String {
    match shell::runs_bash(shell) {
        true => String::new(),
        false => format!(" Commands run under {shell}, not bash: bash-only syntax may not work."),
    }
}

impl Tool for ExecTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".into(),
            description: format!("Execute a bash command in the workspace root. Returns stdout \
                          and stderr. Output is truncated to last 2000 lines or 50KB (whichever is \
                          hit first); past 50KB, a line over 8KB shows only its first and last \
                          2KB. Binary output is named with its size, not shown. If anything is left \
                          out, the output is saved to a stable temp file (very long output keeps \
                          its start and end). A background job (`cmd &`) that \
                          keeps the output open is not waited for: redirect its output to keep it. \
                          Each call runs in a fresh shell in the workspace root: `cd` and \
                          `export` do not carry over to the next call, so chain dependent steps \
                          with `&&` in one command.{} \
                          Optionally provide a timeout in seconds or output_file to write full \
                          combined output to a file and return a concise summary. \
                          Example: {{\"command\": \"ls -la\"}}",
                posix_note(shell::default_shell())
            )
            .into(),
            parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string","description":"Bash command to execute"},"timeout":{"type":"number","description":"Timeout in seconds (optional, capped at configured maximum)"},"output_file":{"type":"string","description":"Path to write full combined stdout/stderr; inline result is a concise summary"}},"required":["command"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();

        Box::pin(async move { self.run_command(&args_str, None).await })
    }
}

#[cfg(test)]
mod ownership_tests;

mod shell;
#[cfg(test)]
use shell::ALLOWED_SHELLS;

#[cfg(test)]
#[path = "../bash_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../bash_output_file_tests.rs"]
mod output_file_tests;

#[cfg(test)]
mod launch_environment_tests;

#[cfg(test)]
mod capture_tests;

#[cfg(test)]
mod saved_output_tests;

#[cfg(test)]
mod read_hint_tests;
