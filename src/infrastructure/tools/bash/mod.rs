// Shell execution tool: impl Tool for ExecTool (bash).
//
// Commands run natively (via the user's shell) in the configured workspace.
// Isolation is delegated to the deployment (e.g. running Quecto in a
// container); the in-process `Sandbox` still confines file/command access and
// can be disabled with `--no-sandbox`.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use crate::infrastructure::tools::truncate::{TruncatedBy, truncate_tail};

/// Default per-command capture cap (10 MiB). Output beyond this is truncated.
const MAX_CAPTURE_BYTES: usize = 10 * 1024 * 1024;
/// Grace window for draining stdout/stderr after a timed-out child is killed.
const STREAM_DRAIN_TIMEOUT_ON_KILL: Duration = Duration::from_millis(250);

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
                });
            }
        };
        let Some(command) = args["command"].as_str().map(str::to_string) else {
            return Ok(ToolResult {
                content: "missing 'command' argument. Example: {\"command\": \"ls -la\"}"
                    .to_string(),
                is_error: true,
                image_blocks: vec![],
            });
        };
        let per_invocation_timeout = parse_timeout(&args);

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

        let source_env = build_source_env(env_overrides);

        let full_command = match &self.command_prefix {
            Some(prefix) => format!("{}\n{}", prefix, command),
            None => command,
        };

        self.spawn_and_wait(&full_command, &source_env, effective_timeout)
            .await
    }

    async fn spawn_and_wait(
        &self,
        command: &str,
        source_env: &HashMap<String, String>,
        timeout_dur: Duration,
    ) -> Result<ToolResult, DomainError> {
        let mut cmd = build_shell_command(&self.workspace, command, source_env);

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| DomainError::Tool(format!("bash failed: {}", e)))?;

        let stdout_task = child
            .stdout
            .take()
            .map(|pipe| tokio::spawn(read_stream_limited(pipe, self.max_capture_bytes)));
        let stderr_task = child
            .stderr
            .take()
            .map(|pipe| tokio::spawn(read_stream_limited(pipe, self.max_capture_bytes)));

        let stream_tasks = StreamTasks {
            stdout_task,
            stderr_task,
        };

        // RED placeholder — process-group reaper added in the GREEN commit.
        run_child_with_timeout(child, stream_tasks, timeout_dur).await
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

fn build_source_env(env_overrides: Option<&HashMap<String, String>>) -> HashMap<String, String> {
    let source: Box<dyn Iterator<Item = (String, String)>> = match env_overrides {
        Some(overrides) => Box::new(overrides.clone().into_iter()),
        None => Box::new(std::env::vars()),
    };
    source.collect()
}

/// Shells that may be selected via the \`SHELL\` environment variable.
///
/// Restricted to well-known system shells to prevent arbitrary binary execution
/// via a crafted or injected \`SHELL\` env var.
const ALLOWED_SHELLS: &[&str] = &[
    "/bin/sh",
    "/bin/bash",
    "/bin/dash",
    "/bin/zsh",
    "/usr/bin/bash",
    "/usr/bin/zsh",
    "/usr/local/bin/bash",
    "/usr/local/bin/zsh",
];

fn build_shell_command(
    workspace: &PathBuf,
    command: &str,
    source_env: &HashMap<String, String>,
) -> tokio::process::Command {
    // Detect the user's shell from $SHELL in the filtered source environment.
    // Validated against an allowlist to prevent arbitrary binary execution.
    let shell = source_env
        .get("SHELL")
        .map(String::as_str)
        .filter(|s| ALLOWED_SHELLS.contains(s))
        .unwrap_or("/bin/sh");

    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(workspace)
        .env_clear();

    // RED placeholder — process-group kill on cancel added in the GREEN commit.

    for (k, v) in source_env {
        cmd.env(k, v);
    }

    if !source_env.contains_key("PATH") {
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
    }

    cmd
}

struct StreamTasks {
    stdout_task: Option<tokio::task::JoinHandle<(String, bool)>>,
    stderr_task: Option<tokio::task::JoinHandle<(String, bool)>>,
}

async fn run_child_with_timeout(
    mut child: tokio::process::Child,
    mut stream_tasks: StreamTasks,
    timeout_dur: Duration,
) -> Result<ToolResult, DomainError> {
    match tokio::time::timeout(timeout_dur, child.wait()).await {
        Ok(Ok(status)) => {
            let output = collect_and_truncate_output(&mut stream_tasks).await;
            Ok(make_exit_result(status, output))
        }
        Ok(Err(e)) => Err(DomainError::Tool(format!("bash failed: {}", e))),
        Err(_) => Ok(handle_timeout(child, stream_tasks, timeout_dur).await),
    }
}

/// Collect stdout + stderr, truncate, and append Quecto-format path hint if needed.
///
/// Truncation notice format (matching Quecto's bash.ts):
/// - Byte-truncated:  `[Showing lines X-Y of Z (50KB limit). Full output: PATH]`
/// - Line-truncated:  `[Showing lines X-Y of Z. Full output: PATH]`
/// - Save fails:      `[Output truncated to last N lines / N bytes]`
async fn collect_and_truncate_output(stream_tasks: &mut StreamTasks) -> String {
    const TAIL_MAX_LINES: usize = 2000;
    const TAIL_MAX_BYTES: usize = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

    let (stdout_raw, _) = await_stream_output(stream_tasks.stdout_task.take()).await;
    let (stderr_raw, _) = await_stream_output(stream_tasks.stderr_task.take()).await;

    let combined = if stderr_raw.is_empty() {
        stdout_raw
    } else if stdout_raw.is_empty() {
        stderr_raw
    } else {
        format!("{}\n{}", stdout_raw, stderr_raw)
    };

    let tr = truncate_tail(&combined, TAIL_MAX_LINES, TAIL_MAX_BYTES);
    if !tr.truncated {
        return tr.content;
    }

    // Compute which lines are shown (tail slice).
    let total = tr.total_lines;
    let shown = tr.output_lines;
    let start_line = total.saturating_sub(shown) + 1;
    let end_line = total;
    let combined_len = combined.len();

    let hint = if let Some(tmp_path) = save_to_temp_file(combined).await {
        let limit_note = if tr.truncated_by == Some(TruncatedBy::Bytes) {
            " (50KB limit)"
        } else {
            ""
        };
        format!(
            "\n[Showing lines {}-{} of {}{}. Full output ({} bytes) saved to: {}]",
            start_line, end_line, total, limit_note, combined_len, tmp_path
        )
    } else {
        format!(
            "\n[Output truncated to last {} lines / {} bytes]",
            TAIL_MAX_LINES, TAIL_MAX_BYTES
        )
    };

    let mut output = tr.content;
    output.push_str(&hint);
    output
}

/// Build a ToolResult from a process exit status.
fn make_exit_result(status: std::process::ExitStatus, content: String) -> ToolResult {
    if status.success() {
        ToolResult {
            content,
            is_error: false,
            image_blocks: vec![],
        }
    } else {
        ToolResult {
            content: format!("exit code {}\n{}", status.code().unwrap_or(-1), content),
            is_error: true,
            image_blocks: vec![],
        }
    }
}

/// Kill the process and drain streams after a timeout.
async fn handle_timeout(
    mut child: tokio::process::Child,
    mut stream_tasks: StreamTasks,
    timeout_dur: Duration,
) -> ToolResult {
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = await_stream_output_with_timeout(
        stream_tasks.stdout_task.take(),
        STREAM_DRAIN_TIMEOUT_ON_KILL,
    )
    .await;
    let _ = await_stream_output_with_timeout(
        stream_tasks.stderr_task.take(),
        STREAM_DRAIN_TIMEOUT_ON_KILL,
    )
    .await;
    ToolResult {
        content: format!("command timed out after {}s", timeout_dur.as_secs()),
        is_error: true,
        image_blocks: vec![],
    }
}

async fn read_stream_limited<R>(mut pipe: R, max_capture_bytes: usize) -> (String, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let mut collected = Vec::new();
    let mut chunk = [0_u8; 8192];
    let mut truncated = false;

    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                let remaining = max_capture_bytes.saturating_sub(collected.len());
                if remaining > 0 {
                    let keep = remaining.min(n);
                    collected.extend_from_slice(&chunk[..keep]);
                }
                if n > remaining {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }

    (String::from_utf8_lossy(&collected).into_owned(), truncated)
}

async fn await_stream_output(
    task: Option<tokio::task::JoinHandle<(String, bool)>>,
) -> (String, bool) {
    match task {
        Some(handle) => (handle.await).unwrap_or_default(),
        None => (String::new(), false),
    }
}

async fn await_stream_output_with_timeout(
    task: Option<tokio::task::JoinHandle<(String, bool)>>,
    timeout: Duration,
) -> (String, bool) {
    if let Some(handle) = task {
        match tokio::time::timeout(timeout, handle).await {
            Ok(join) => join.unwrap_or_default(),
            Err(_) => (String::new(), false),
        }
    } else {
        (String::new(), false)
    }
}

/// Save content to a temp file asynchronously and return the path.
async fn save_to_temp_file(content: String) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().ok()?;
        f.write_all(content.as_bytes()).ok()?;
        let (_, path) = f.keep().ok()?;
        Some(path.display().to_string())
    })
    .await
    .ok()?
}

impl Tool for ExecTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".into(),
            description: "Execute a bash command in the current working directory. Returns stdout \
                          and stderr. Output is truncated to last 2000 lines or 50KB (whichever is \
                          hit first). If truncated, full output is saved to a temp file. \
                          Optionally provide a timeout in seconds. \
                          Example: {\"command\": \"ls -la\"}"
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string","description":"Bash command to execute"},"timeout":{"type":"number","description":"Timeout in seconds (optional, capped at configured maximum)"}},"required":["command"]}"#.into(),
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
#[path = "../bash_tests.rs"]
mod tests;
