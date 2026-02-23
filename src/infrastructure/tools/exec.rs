// Shell execution tool: impl Tool for ExecTool.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

/// Default timeout for command execution (30 seconds).
const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(30);

/// Prefix for environment variables that should be stripped from child processes.
const SECRET_ENV_PREFIX: &str = "QUECTO_";

/// Maximum bytes captured per stream (stdout and stderr) to avoid unbounded memory growth.
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
const STREAM_DRAIN_TIMEOUT_ON_KILL: Duration = Duration::from_millis(250);

/// Runtime isolation mode for the exec tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecIsolationMode {
    Native,
    Nsjail,
}

/// Configuration for nsjail execution.
#[derive(Debug, Clone)]
pub struct NsjailOptions {
    pub binary: String,
    pub network_passthrough: bool,
    pub memory_limit_mb: Option<u64>,
    pub pid_limit: Option<u64>,
    pub cpu_time_limit_secs: Option<u64>,
    pub wall_time_limit_secs: Option<u64>,
}

impl Default for NsjailOptions {
    fn default() -> Self {
        Self {
            binary: "nsjail".to_string(),
            network_passthrough: false,
            memory_limit_mb: None,
            pid_limit: None,
            cpu_time_limit_secs: None,
            wall_time_limit_secs: None,
        }
    }
}

/// Runtime options for ExecTool.
#[derive(Debug, Clone)]
pub struct ExecOptions {
    pub timeout: Duration,
    pub max_capture_bytes: usize,
    pub isolation_mode: ExecIsolationMode,
    pub nsjail: NsjailOptions,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_EXEC_TIMEOUT,
            max_capture_bytes: MAX_CAPTURE_BYTES,
            isolation_mode: ExecIsolationMode::Native,
            nsjail: NsjailOptions::default(),
        }
    }
}

/// Tool that executes shell commands within the workspace.
pub struct ExecTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    timeout: Duration,
    max_capture_bytes: usize,
    mode: ExecIsolationMode,
    nsjail: NsjailOptions,
    startup_warning: Option<String>,
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
    /// Create a new exec tool with default timeout.
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self::with_options(workspace, sandbox, ExecOptions::default())
    }

    /// Create a new exec tool with a custom timeout.
    pub fn with_timeout(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>, timeout: Duration) -> Self {
        let opts = ExecOptions {
            timeout,
            ..ExecOptions::default()
        };
        Self::with_options(workspace, sandbox, opts)
    }

    /// Create a new exec tool with custom timeout and output capture cap.
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

    /// Create a new exec tool with explicit options.
    pub fn with_options(
        workspace: Arc<PathBuf>,
        sandbox: Arc<Sandbox>,
        options: ExecOptions,
    ) -> Self {
        let mut warning = None;
        let mut mode = options.isolation_mode;
        if mode == ExecIsolationMode::Nsjail && !binary_exists(&options.nsjail.binary) {
            mode = ExecIsolationMode::Native;
            warning = Some(format!(
                "nsjail binary '{}' not found; falling back to native exec",
                options.nsjail.binary
            ));
            tracing::warn!(target: "exec", "{}", warning.as_deref().unwrap_or_default());
        }
        Self {
            workspace,
            sandbox,
            timeout: options.timeout,
            max_capture_bytes: options.max_capture_bytes,
            mode,
            nsjail: options.nsjail,
            startup_warning: warning,
        }
    }

    /// Get the configured timeout duration.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Current runtime mode (may differ from requested mode after fallback).
    pub fn mode(&self) -> ExecIsolationMode {
        self.mode
    }

    /// Startup warning set when fallback occurred.
    pub fn startup_warning(&self) -> Option<&str> {
        self.startup_warning.as_deref()
    }

    /// Execute a command with custom environment variables.
    /// Environment variables prefixed with `QUECTO_` are stripped from the child process.
    pub fn execute_with_env(
        &self,
        arguments: &str,
        env_overrides: &HashMap<String, String>,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        let env_overrides = env_overrides.clone();

        Box::pin(async move { self.run_command(&args_str, Some(&env_overrides)).await })
    }

    /// Core execution logic shared by both `Tool::execute` and `execute_with_env`.
    ///
    /// When `env_overrides` is `Some`, those variables are used as the child's environment
    /// (after stripping `QUECTO_` prefixed keys). When `None`, the current process environment
    /// is inherited (also with `QUECTO_` keys stripped).
    ///
    /// In both cases `env_clear()` is called first to ensure a clean slate, then allowed
    /// variables are selectively re-added.
    async fn run_command(
        &self,
        arguments: &str,
        env_overrides: Option<&HashMap<String, String>>,
    ) -> Result<ToolResult, DomainError> {
        let command = extract_command(arguments)?;

        // Validate command against sandbox
        self.sandbox
            .validate_command(&command)
            .map_err(|e| DomainError::Security(e.to_string()))?;

        let source_env = build_source_env(env_overrides);
        let mut cmd = build_command(self, &command, &source_env);

        // Spawn the child process so we can kill it on timeout.
        // We use spawn + wait (not output) so we retain a handle to kill the process.
        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| DomainError::Tool(format!("exec failed: {}", e)))?;

        // Start draining stdout/stderr immediately so child processes do not block on full pipes.
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

        run_child_with_timeout(child, stream_tasks, self.timeout, self.max_capture_bytes).await
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
    match env_overrides {
        Some(overrides) => overrides.clone(),
        None => std::env::vars().collect(),
    }
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

fn build_command(
    tool: &ExecTool,
    command: &str,
    source_env: &HashMap<String, String>,
) -> tokio::process::Command {
    if tool.mode == ExecIsolationMode::Nsjail {
        return build_nsjail_command(&tool.workspace, command, source_env, &tool.nsjail);
    }
    build_shell_command(&tool.workspace, command, source_env)
}

fn build_nsjail_command(
    workspace: &Path,
    command: &str,
    source_env: &HashMap<String, String>,
    options: &NsjailOptions,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(&options.binary);
    cmd.arg("--quiet")
        .arg("--mode")
        .arg("o")
        .arg("--cwd")
        .arg("/workspace")
        .arg("--bindmount")
        .arg(format!("{}:/workspace", workspace.display()));

    if !options.network_passthrough {
        cmd.arg("--clone_newnet");
    }
    if let Some(mem) = options.memory_limit_mb {
        cmd.arg("--cgroup_mem_max")
            .arg((mem * 1024 * 1024).to_string());
    }
    if let Some(pid) = options.pid_limit {
        cmd.arg("--cgroup_pids_max").arg(pid.to_string());
    }
    if let Some(cpu) = options.cpu_time_limit_secs {
        cmd.arg("--rlimit_cpu").arg(cpu.to_string());
    }
    if let Some(wall) = options.wall_time_limit_secs {
        cmd.arg("--time_limit").arg(wall.to_string());
    }

    cmd.arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(workspace)
        .env_clear();

    for (k, v) in source_env {
        if !k.starts_with(SECRET_ENV_PREFIX) {
            cmd.env(k, v);
        }
    }
    if !source_env.contains_key("PATH")
        && let Ok(path) = std::env::var("PATH")
    {
        cmd.env("PATH", path);
    }
    cmd
}

fn binary_exists(binary: &str) -> bool {
    let path = Path::new(binary);
    if path.components().count() > 1 {
        return path.exists();
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(binary);
            if candidate.exists() {
                return true;
            }
        }
    }
    false
}

async fn run_child_with_timeout(
    mut child: tokio::process::Child,
    mut stream_tasks: StreamTasks,
    timeout_dur: Duration,
    max_capture_bytes: usize,
) -> Result<ToolResult, DomainError> {
    match tokio::time::timeout(timeout_dur, child.wait()).await {
        Ok(Ok(status)) => {
            let (stdout, stdout_truncated) =
                await_stream_output(stream_tasks.stdout_task.take()).await;
            let (stderr, stderr_truncated) =
                await_stream_output(stream_tasks.stderr_task.take()).await;

            let stdout = annotate_truncation(stdout, stdout_truncated, "stdout", max_capture_bytes);
            let stderr = annotate_truncation(stderr, stderr_truncated, "stderr", max_capture_bytes);

            if status.success() {
                Ok(ToolResult {
                    content: stdout,
                    is_error: false,
                })
            } else {
                Ok(ToolResult {
                    content: format!(
                        "exit code {}\nstdout: {}\nstderr: {}",
                        status, stdout, stderr
                    ),
                    is_error: true,
                })
            }
        }
        Ok(Err(e)) => Err(DomainError::Tool(format!("exec failed: {}", e))),
        Err(_) => {
            // Timeout: kill the child process to prevent orphan leak
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
            Ok(ToolResult {
                content: format!("command timed out after {}s", timeout_dur.as_secs()),
                is_error: true,
            })
        }
    }
}

struct StreamTasks {
    stdout_task: Option<tokio::task::JoinHandle<(String, bool)>>,
    stderr_task: Option<tokio::task::JoinHandle<(String, bool)>>,
}

/// Read bytes from a process stream with a fixed capture cap.
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

fn annotate_truncation(
    mut content: String,
    truncated: bool,
    stream_name: &str,
    max_capture_bytes: usize,
) -> String {
    if truncated {
        content.push_str(&format!(
            "\n[{} output truncated at {} bytes]",
            stream_name, max_capture_bytes
        ));
    }
    content
}

impl Tool for ExecTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "exec".to_string(),
            description: "Execute a shell command in the workspace directory".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string","description":"Shell command to execute"}},"required":["command"]}"#.to_string(),
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_exec(restrict: bool) -> (ExecTool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), restrict);
        let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));
        (tool, tmp)
    }

    #[tokio::test]
    async fn test_exec_echo() {
        let (tool, _tmp) = test_exec(false);
        let result = tool.execute(r#"{"command": "echo hello"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("hello"));
    }

    #[tokio::test]
    async fn test_exec_dangerous_command_blocked() {
        let (tool, _tmp) = test_exec(false);
        let result = tool.execute(r#"{"command": "rm -rf /"}"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_exec_missing_command_arg() {
        let (tool, _tmp) = test_exec(false);
        let result = tool.execute(r#"{}"#).await;
        assert!(result.is_err());
    }

    // --- Sandbox hardening: timeout tests ---

    #[tokio::test]
    async fn test_exec_timeout_kills_long_command() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let tool = ExecTool::with_timeout(
            Arc::new(tmp.path().to_path_buf()),
            Arc::new(sandbox),
            Duration::from_secs(1),
        );

        let result = tool.execute(r#"{"command": "sleep 60"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("timed out"));
    }

    #[tokio::test]
    async fn test_exec_command_within_timeout() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let tool = ExecTool::with_timeout(
            Arc::new(tmp.path().to_path_buf()),
            Arc::new(sandbox),
            Duration::from_secs(5),
        );

        let result = tool.execute(r#"{"command": "echo fast"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("fast"));
    }

    #[tokio::test]
    async fn test_exec_large_output_completes_without_timeout() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let tool = ExecTool::with_timeout(
            Arc::new(tmp.path().to_path_buf()),
            Arc::new(sandbox),
            Duration::from_secs(1),
        );

        let result = tool
            .execute(r#"{"command": "printf 'x%.0s' {1..100000}"}"#)
            .await
            .unwrap();

        assert!(
            !result.is_error,
            "expected large output command to complete, got: {}",
            result.content
        );
        assert!(result.content.contains('x'));
    }

    #[test]
    fn test_default_timeout_is_30_seconds() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));
        assert_eq!(tool.timeout().as_secs(), 30);
    }

    // --- Sandbox hardening: env sanitization tests ---

    #[tokio::test]
    async fn test_exec_strips_quecto_env_vars() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));

        let mut env_vars = HashMap::new();
        env_vars.insert(
            "QUECTO_PROVIDERS_OPENAI_API_KEY".to_string(),
            "sk-secret".to_string(),
        );
        env_vars.insert("HOME".to_string(), "/home/testuser".to_string());

        let result = tool
            .execute_with_env(
                r#"{"command": "printenv QUECTO_PROVIDERS_OPENAI_API_KEY"}"#,
                &env_vars,
            )
            .await
            .unwrap();
        // The QUECTO_ var should not be in the output
        assert!(!result.content.contains("sk-secret"));
    }

    #[tokio::test]
    async fn test_exec_preserves_non_secret_env_vars() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));

        let mut env_vars = HashMap::new();
        env_vars.insert("HOME".to_string(), "/home/user".to_string());

        let result = tool
            .execute_with_env(r#"{"command": "printenv HOME"}"#, &env_vars)
            .await
            .unwrap();
        assert!(result.content.contains("/home/user"));
    }

    #[test]
    fn test_debug_format() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));
        let debug = format!("{:?}", tool);
        assert!(debug.contains("ExecTool"));
    }

    #[test]
    fn test_nsjail_mode_selected_when_binary_exists() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let opts = ExecOptions {
            isolation_mode: ExecIsolationMode::Nsjail,
            nsjail: NsjailOptions {
                binary: "sh".to_string(),
                ..NsjailOptions::default()
            },
            ..ExecOptions::default()
        };
        let tool =
            ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);
        assert_eq!(tool.mode(), ExecIsolationMode::Nsjail);
        assert!(tool.startup_warning().is_none());
    }

    #[test]
    fn test_nsjail_falls_back_when_binary_missing() {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
        let opts = ExecOptions {
            isolation_mode: ExecIsolationMode::Nsjail,
            nsjail: NsjailOptions {
                binary: "definitely-not-a-real-binary".to_string(),
                ..NsjailOptions::default()
            },
            ..ExecOptions::default()
        };
        let tool =
            ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);
        assert_eq!(tool.mode(), ExecIsolationMode::Native);
        assert!(
            tool.startup_warning()
                .unwrap_or_default()
                .contains("falling back to native")
        );
    }
}
