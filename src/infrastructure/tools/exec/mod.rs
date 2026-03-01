// Shell execution tool: impl Tool for ExecTool (bash).

mod nsjail;

pub use nsjail::{ExecIsolationMode, NsjailOptions};

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;

#[cfg(any(test, feature = "test-support"))]
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use nsjail::{
    DEFAULT_EXEC_TIMEOUT, EXEC_ENV_ALLOWLIST, ExecIsolationMode as Mode, MAX_CAPTURE_BYTES,
    NsjailConfig, SECRET_ENV_PREFIX, STREAM_DRAIN_TIMEOUT_ON_KILL, build_nsjail_command,
    resolve_nsjail_binary, resolve_ro_bindmounts, resolve_ro_dev_files, resolve_ro_etc_files,
};

#[derive(Debug, Clone)]
pub struct ExecOptions {
    pub timeout: Duration,
    pub max_capture_bytes: usize,
    pub isolation_mode: ExecIsolationMode,
    pub allow_native_fallback: bool,
    pub nsjail: NsjailOptions,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_EXEC_TIMEOUT,
            max_capture_bytes: MAX_CAPTURE_BYTES,
            isolation_mode: ExecIsolationMode::Native,
            allow_native_fallback: false,
            nsjail: NsjailOptions::default(),
        }
    }
}

pub struct ExecTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    timeout: Duration,
    max_capture_bytes: usize,
    mode: ExecIsolationMode,
    nsjail: NsjailOptions,
    /// System paths resolved at construction time to avoid per-execution stat() calls.
    ro_bindmounts: Vec<&'static str>,
    ro_etc_files: Vec<&'static str>,
    ro_dev_files: Vec<&'static str>,
    startup_warning: Option<String>,
    startup_error: Option<String>,
}

impl std::fmt::Debug for ExecTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecTool")
            .field("workspace", &self.workspace)
            .field("timeout", &self.timeout)
            .field("max_capture_bytes", &self.max_capture_bytes)
            .field("mode", &self.mode)
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

    pub fn with_limits(
        workspace: Arc<PathBuf>,
        sandbox: Arc<Sandbox>,
        timeout: Duration,
        max_capture_bytes: usize,
    ) -> Self {
        let opts = ExecOptions {
            timeout,
            max_capture_bytes,
            ..ExecOptions::default()
        };
        Self::with_options(workspace, sandbox, opts)
    }

    pub fn with_options(
        workspace: Arc<PathBuf>,
        sandbox: Arc<Sandbox>,
        mut options: ExecOptions,
    ) -> Self {
        let mut warning = None;
        let mut startup_error = None;
        let mut mode = options.isolation_mode;
        if mode == Mode::Nsjail {
            if options.nsjail.memory_limit_mb.is_none()
                && options.nsjail.pid_limit.is_none()
                && options.nsjail.cpu_time_limit_secs.is_none()
                && options.nsjail.wall_time_limit_secs.is_none()
            {
                tracing::warn!(
                    target: "exec",
                    "nsjail isolation is active but all resource limits are disabled."
                );
            }
            if let Some(resolved_binary) = resolve_nsjail_binary(&options.nsjail.binary) {
                options.nsjail.binary = resolved_binary;
            } else {
                let missing = format!(
                    "nsjail binary '{}' is not available or not executable",
                    options.nsjail.binary
                );
                if options.allow_native_fallback {
                    mode = Mode::Native;
                    warning = Some(format!("{}; falling back to native exec", missing));
                    tracing::warn!(target: "exec", "{}", warning.as_deref().unwrap_or_default());
                } else {
                    startup_error = Some(format!(
                        "{}; set tools.exec.allow_native_fallback=true to permit native fallback",
                        missing
                    ));
                    tracing::error!(target: "exec", "{}", startup_error.as_deref().unwrap_or_default());
                }
            }
        }
        let ro_bindmounts = resolve_ro_bindmounts();
        let ro_etc_files = resolve_ro_etc_files();
        let ro_dev_files = resolve_ro_dev_files();
        Self {
            workspace,
            sandbox,
            timeout: options.timeout,
            max_capture_bytes: options.max_capture_bytes,
            mode,
            nsjail: options.nsjail,
            ro_bindmounts,
            ro_etc_files,
            ro_dev_files,
            startup_warning: warning,
            startup_error,
        }
    }

    /// Construct with options but skip nsjail binary path validation.
    ///
    /// Intended **only** for tests where a fake nsjail script lives outside
    /// the trusted system directories.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_options_trusted(
        workspace: Arc<PathBuf>,
        sandbox: Arc<Sandbox>,
        options: ExecOptions,
    ) -> Self {
        let ro_bindmounts = resolve_ro_bindmounts();
        let ro_etc_files = resolve_ro_etc_files();
        let ro_dev_files = resolve_ro_dev_files();
        Self {
            workspace,
            sandbox,
            timeout: options.timeout,
            max_capture_bytes: options.max_capture_bytes,
            mode: options.isolation_mode,
            nsjail: options.nsjail,
            ro_bindmounts,
            ro_etc_files,
            ro_dev_files,
            startup_warning: None,
            startup_error: None,
        }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn mode(&self) -> ExecIsolationMode {
        self.mode
    }

    pub fn startup_warning(&self) -> Option<&str> {
        self.startup_warning.as_deref()
    }

    pub fn startup_error(&self) -> Option<&str> {
        self.startup_error.as_deref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn nsjail_options(&self) -> &NsjailOptions {
        &self.nsjail
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn build_nsjail_command_for_testing(
        &self,
        workspace: &Path,
        command: &str,
    ) -> tokio::process::Command {
        let source_env = HashMap::new();
        let config = NsjailConfig {
            options: &self.nsjail,
            ro_dirs: &self.ro_bindmounts,
            ro_etc_files: &self.ro_etc_files,
            ro_dev_files: &self.ro_dev_files,
        };
        build_nsjail_command(workspace, command, &source_env, &config)
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
        if let Some(startup_error) = &self.startup_error {
            return Err(DomainError::Config(startup_error.clone()));
        }

        let command = extract_command(arguments)?;

        self.sandbox
            .validate_command(&command)
            .map_err(|e| DomainError::Security(e.to_string()))?;

        let source_env = build_source_env(env_overrides);

        self.spawn_and_wait(&command, &source_env, self.mode).await
    }

    async fn spawn_and_wait(
        &self,
        command: &str,
        source_env: &HashMap<String, String>,
        mode: ExecIsolationMode,
    ) -> Result<ToolResult, DomainError> {
        let mut cmd = match mode {
            Mode::Nsjail => {
                let config = NsjailConfig {
                    options: &self.nsjail,
                    ro_dirs: &self.ro_bindmounts,
                    ro_etc_files: &self.ro_etc_files,
                    ro_dev_files: &self.ro_dev_files,
                };
                build_nsjail_command(&self.workspace, command, source_env, &config)
            }
            Mode::Native => build_shell_command(&self.workspace, command, source_env),
        };

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

        run_child_with_timeout(child, stream_tasks, self.timeout).await
    }
}

fn extract_command(arguments: &str) -> Result<String, DomainError> {
    let args: serde_json::Value =
        serde_json::from_str(arguments).map_err(|e| DomainError::Tool(e.to_string()))?;
    args["command"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| DomainError::Tool("missing 'command' argument".to_string()))
}

fn build_source_env(env_overrides: Option<&HashMap<String, String>>) -> HashMap<String, String> {
    let source: Box<dyn Iterator<Item = (String, String)>> = match env_overrides {
        Some(overrides) => Box::new(overrides.clone().into_iter()),
        None => Box::new(std::env::vars()),
    };
    source.filter(|(k, _)| is_allowed_exec_env_key(k)).collect()
}

fn is_allowed_exec_env_key(key: &str) -> bool {
    !key.starts_with(SECRET_ENV_PREFIX)
        && (EXEC_ENV_ALLOWLIST.contains(&key) || key.starts_with("LC_"))
}

fn build_shell_command(
    workspace: &PathBuf,
    command: &str,
    source_env: &HashMap<String, String>,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(workspace)
        .env_clear();

    for (k, v) in source_env {
        if !k.starts_with(SECRET_ENV_PREFIX) {
            cmd.env(k, v);
        }
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

/// Collect stdout + stderr, truncate, and append path hint if needed.
async fn collect_and_truncate_output(stream_tasks: &mut StreamTasks) -> String {
    const TAIL_MAX_LINES: usize = 2000;
    const TAIL_MAX_BYTES: usize = 50 * 1024;

    let (stdout_raw, _) = await_stream_output(stream_tasks.stdout_task.take()).await;
    let (stderr_raw, _) = await_stream_output(stream_tasks.stderr_task.take()).await;

    let combined = if stderr_raw.is_empty() {
        stdout_raw
    } else if stdout_raw.is_empty() {
        stderr_raw
    } else {
        format!("{}\n{}", stdout_raw, stderr_raw)
    };

    let (truncated_output, was_truncated) =
        truncate_tail_output(&combined, TAIL_MAX_LINES, TAIL_MAX_BYTES);
    let mut content = truncated_output;

    if was_truncated {
        let combined_len = combined.len();
        let hint = if let Some(tmp_path) = save_to_temp_file(combined).await {
            format!(
                "\n[Output truncated. Full output ({} bytes) saved to: {}]",
                combined_len, tmp_path
            )
        } else {
            format!(
                "\n[Output truncated to last {} lines / {} bytes]",
                TAIL_MAX_LINES, TAIL_MAX_BYTES
            )
        };
        content.push_str(&hint);
    }
    content
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

    (String::from_utf8_lossy(&collected).to_string(), truncated)
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

/// Truncate `content` to at most `max_lines` tail-lines or `max_bytes` bytes.
/// Pi-parity: the *last* max_lines/max_bytes are kept, not the first.
fn truncate_tail_output(content: &str, max_lines: usize, max_bytes: usize) -> (String, bool) {
    let mut kept_lines: Vec<&str> = Vec::with_capacity(max_lines.min(4096));
    let mut byte_count = 0usize;

    for line in content.lines().rev() {
        let line_bytes = line.len() + 1;
        if byte_count + line_bytes > max_bytes || kept_lines.len() >= max_lines {
            if kept_lines.is_empty() {
                break;
            }
            kept_lines.reverse();
            return (kept_lines.join("\n"), true);
        }
        kept_lines.push(line);
        byte_count += line_bytes;
    }

    if kept_lines.is_empty() && !content.is_empty() {
        let raw_start = content.len().saturating_sub(max_bytes);
        let start = (raw_start..=content.len())
            .find(|&i| content.is_char_boundary(i))
            .unwrap_or(0);
        return (content[start..].to_string(), true);
    }

    (content.to_string(), false)
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
            name: "bash".to_string(),
            description: "Execute a bash command in the current working directory. Returns stdout \
                          and stderr. Output is truncated to last 2000 lines or 50KB (whichever is \
                          hit first). If truncated, full output is saved to a temp file."
                .to_string(),
            parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string","description":"Bash command to execute"}},"required":["command"]}"#.to_string(),
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
#[path = "../exec_tests.rs"]
mod tests;
