// Shell execution tool: impl Tool for ExecTool.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(30);
const SECRET_ENV_PREFIX: &str = "QUECTO_";
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
const STREAM_DRAIN_TIMEOUT_ON_KILL: Duration = Duration::from_millis(250);
const DEFAULT_NSJAIL_MEMORY_LIMIT_MB: u64 = 512;
const DEFAULT_NSJAIL_PID_LIMIT: u64 = 256;
const DEFAULT_NSJAIL_CPU_TIME_LIMIT_SECS: u64 = 30;
const DEFAULT_NSJAIL_WALL_TIME_LIMIT_SECS: u64 = 30;
const TRUSTED_NSJAIL_PATHS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/usr/local/bin"];
const EXEC_ENV_ALLOWLIST: &[&str] = &[
    "HOME", "PATH", "LANG", "TZ", "TERM", "SHELL", "USER", "LOGNAME", "TMPDIR",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecIsolationMode {
    Native,
    Nsjail,
}

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
            memory_limit_mb: Some(DEFAULT_NSJAIL_MEMORY_LIMIT_MB),
            pid_limit: Some(DEFAULT_NSJAIL_PID_LIMIT),
            cpu_time_limit_secs: Some(DEFAULT_NSJAIL_CPU_TIME_LIMIT_SECS),
            wall_time_limit_secs: Some(DEFAULT_NSJAIL_WALL_TIME_LIMIT_SECS),
        }
    }
}

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
    allow_native_fallback: bool,
    nsjail: NsjailOptions,
    startup_warning: Option<String>,
    startup_error: Option<String>,
    /// Latched to `true` after the first runtime cgroup fallback succeeds,
    /// so subsequent calls skip the doomed nsjail attempt entirely.
    cgroup_fallback_latched: AtomicBool,
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
        if mode == ExecIsolationMode::Nsjail {
            if let Some(resolved_binary) = resolve_nsjail_binary(&options.nsjail.binary) {
                options.nsjail.binary = resolved_binary;
            } else {
                let missing = format!(
                    "nsjail binary '{}' is not available or not executable",
                    options.nsjail.binary
                );
                if options.allow_native_fallback {
                    mode = ExecIsolationMode::Native;
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
        Self {
            workspace,
            sandbox,
            timeout: options.timeout,
            max_capture_bytes: options.max_capture_bytes,
            mode,
            allow_native_fallback: options.allow_native_fallback,
            nsjail: options.nsjail,
            startup_warning: warning,
            startup_error,
            cgroup_fallback_latched: AtomicBool::new(false),
        }
    }

    /// Construct with options but skip nsjail binary path validation.
    ///
    /// # Safety (logical)
    /// The caller asserts the binary path is safe. Intended **only** for
    /// tests where a fake nsjail script lives outside the trusted system
    /// directories. Absent from release builds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_options_trusted(
        workspace: Arc<PathBuf>,
        sandbox: Arc<Sandbox>,
        options: ExecOptions,
    ) -> Self {
        Self {
            workspace,
            sandbox,
            timeout: options.timeout,
            max_capture_bytes: options.max_capture_bytes,
            mode: options.isolation_mode,
            allow_native_fallback: options.allow_native_fallback,
            nsjail: options.nsjail,
            startup_warning: None,
            startup_error: None,
            cgroup_fallback_latched: AtomicBool::new(false),
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

        // If a previous cgroup failure already latched the fallback, skip nsjail.
        let effective_mode = if self.cgroup_fallback_latched.load(Ordering::Relaxed) {
            ExecIsolationMode::Native
        } else {
            self.mode
        };

        let result = self
            .spawn_and_wait(&command, &source_env, effective_mode)
            .await?;

        // If nsjail failed with a cgroup-related error, retry with native exec
        // and latch the fallback so subsequent calls skip nsjail entirely.
        // NOTE: This silently downgrades from kernel-level nsjail isolation to
        // software-only sandbox denylist. Operators should monitor for this log
        // message and fix the underlying cgroup configuration.
        if effective_mode == ExecIsolationMode::Nsjail
            && self.allow_native_fallback
            && result.is_error
            && is_nsjail_cgroup_failure(&result.content)
        {
            tracing::error!(
                target: "exec",
                command = command,
                "nsjail cgroup setup failed; falling back to native exec. \
                 All subsequent commands will run WITHOUT nsjail isolation. \
                 Fix cgroup configuration or set tools.exec.isolation=native."
            );
            self.cgroup_fallback_latched.store(true, Ordering::Relaxed);
            return self
                .spawn_and_wait(&command, &source_env, ExecIsolationMode::Native)
                .await;
        }

        Ok(result)
    }

    async fn spawn_and_wait(
        &self,
        command: &str,
        source_env: &HashMap<String, String>,
        mode: ExecIsolationMode,
    ) -> Result<ToolResult, DomainError> {
        let mut cmd = match mode {
            ExecIsolationMode::Nsjail => {
                build_nsjail_command(&self.workspace, command, source_env, &self.nsjail)
            }
            ExecIsolationMode::Native => build_shell_command(&self.workspace, command, source_env),
        };

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| DomainError::Tool(format!("exec failed: {}", e)))?;

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

/// Check whether an nsjail error result indicates a cgroup setup failure.
///
/// Only inspects the stderr portion of the formatted error output to avoid
/// false positives from user command stdout that mentions cgroup keywords.
/// Also pre-filters on nsjail's internal exit code 255 (distinct from the
/// child process exit code).
///
/// Patterns validated against nsjail 3.4 (2025-01). Re-audit on version bumps.
fn is_nsjail_cgroup_failure(content: &str) -> bool {
    // Pre-filter: nsjail uses exit code 255 for internal failures.
    // The formatted output starts with "exit code exit status: 255" on Linux.
    let first_line = content.lines().next().unwrap_or("");
    if !first_line.contains("255") {
        return false;
    }

    // Extract only the stderr portion to avoid matching user stdout.
    // Format: "exit code {status}\nstdout: {stdout}\nstderr: {stderr}"
    let stderr_section = content
        .find("\nstderr: ")
        .map(|idx| &content[idx..])
        .unwrap_or(content);

    // Limit scan to last 4 KiB to avoid allocating a lowercased copy
    // of potentially large (up to 1 MiB) output.
    let tail = if stderr_section.len() > 4096 {
        &stderr_section[stderr_section.len() - 4096..]
    } else {
        stderr_section
    };
    let lower = tail.to_lowercase();

    lower.contains("createcgroup")
        || lower.contains("couldn't initialize cgroup")
        || (lower.contains("cgroup") && lower.contains("no such file or directory"))
        || (lower.contains("cgroup") && lower.contains("permission denied"))
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

    // Auto-detect cgroup version so that nsjail uses the correct mount paths.
    // On cgroup v2 hosts, nsjail's default v1 paths (/sys/fs/cgroup/memory etc.)
    // do not exist, causing cgroup setup failures.
    cmd.arg("--detect_cgroupv2");

    if options.network_passthrough {
        cmd.arg("--disable_clone_newnet");
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

fn resolve_nsjail_binary(binary: &str) -> Option<String> {
    let path = Path::new(binary);
    if path.components().count() > 1 {
        if !path.is_absolute() {
            return None;
        }
        let canonical = std::fs::canonicalize(path).ok()?;
        if is_trusted_nsjail_binary_path(&canonical) && is_executable_file(&canonical) {
            return Some(canonical.to_string_lossy().to_string());
        }
        return None;
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if !is_trusted_nsjail_search_dir(&dir) {
                continue;
            }
            let candidate = dir.join(binary);
            let canonical = std::fs::canonicalize(&candidate).ok();
            if let Some(canonical) = canonical
                && is_trusted_nsjail_binary_path(&canonical)
                && is_executable_file(&canonical)
            {
                return Some(canonical.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn is_trusted_nsjail_search_dir(path: &Path) -> bool {
    TRUSTED_NSJAIL_PATHS
        .iter()
        .any(|allowed| path == Path::new(allowed))
}

fn is_trusted_nsjail_binary_path(path: &Path) -> bool {
    TRUSTED_NSJAIL_PATHS
        .iter()
        .map(Path::new)
        .any(|root| path.starts_with(root))
}

fn is_executable_file(path: &Path) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(_) => return false,
    };
    if !meta.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        meta.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
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
#[path = "exec_tests.rs"]
mod tests;
