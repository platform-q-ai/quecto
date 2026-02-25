// Shell execution tool: impl Tool for ExecTool.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

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
const NSJAIL_HELP_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
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
    pub die_with_parent: bool,
    pub allow_without_die_with_parent: bool,
    /// Additional directories whose binaries are trusted for nsjail resolution.
    /// Production code leaves this empty. Tests add temp directories here so that
    /// fake nsjail scripts in non-standard locations can pass validation.
    pub additional_trusted_paths: Vec<PathBuf>,
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
            die_with_parent: true,
            allow_without_die_with_parent: false,
            additional_trusted_paths: Vec::new(),
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
    nsjail: NsjailOptions,
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
        if mode == ExecIsolationMode::Nsjail {
            if let Some(resolved_binary) = resolve_nsjail_binary(
                &options.nsjail.binary,
                &options.nsjail.additional_trusted_paths,
            ) {
                options.nsjail.binary = resolved_binary;
                if options.nsjail.die_with_parent
                    && !nsjail_supports_flag(&options.nsjail.binary, "--die_with_parent")
                {
                    if options.nsjail.allow_without_die_with_parent {
                        options.nsjail.die_with_parent = false;
                        warning = Some(format!(
                            "nsjail binary '{}' does not support --die_with_parent; continuing without it",
                            options.nsjail.binary
                        ));
                        tracing::warn!(target: "exec", "{}", warning.as_deref().unwrap_or_default());
                    } else {
                        startup_error = Some(format!(
                            "nsjail binary '{}' does not support required --die_with_parent; set tools.exec.allow_without_die_with_parent=true to allow downgrade",
                            options.nsjail.binary
                        ));
                        tracing::error!(target: "exec", "{}", startup_error.as_deref().unwrap_or_default());
                    }
                }
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
            nsjail: options.nsjail,
            startup_warning: warning,
            startup_error,
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
        let mut cmd = build_command(self, &command, &source_env);

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

    if options.die_with_parent {
        cmd.arg("--die_with_parent");
    }

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

fn resolve_nsjail_binary(binary: &str, extra_trusted: &[PathBuf]) -> Option<String> {
    let is_extra_trusted_path = |p: &Path| extra_trusted.iter().any(|root| p.starts_with(root));
    let is_extra_trusted_dir = |p: &Path| extra_trusted.iter().any(|root| p == root);
    let path = Path::new(binary);
    if path.components().count() > 1 {
        if !path.is_absolute() {
            return None;
        }
        let canonical = std::fs::canonicalize(path).ok()?;
        let trusted =
            is_trusted_nsjail_binary_path(&canonical) || is_extra_trusted_path(&canonical);
        if trusted && is_executable_file(&canonical) {
            if is_extra_trusted_path(&canonical) {
                tracing::debug!(
                    target: "exec",
                    binary = %canonical.display(),
                    "nsjail binary resolved via additional trusted path"
                );
            }
            return Some(canonical.to_string_lossy().to_string());
        }
        return None;
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if !is_trusted_nsjail_search_dir(&dir) && !is_extra_trusted_dir(&dir) {
                continue;
            }
            let candidate = dir.join(binary);
            let canonical = std::fs::canonicalize(&candidate).ok();
            if let Some(canonical) = canonical
                && (is_trusted_nsjail_binary_path(&canonical) || is_extra_trusted_path(&canonical))
                && is_executable_file(&canonical)
            {
                if is_extra_trusted_path(&canonical) {
                    tracing::debug!(
                        target: "exec",
                        binary = %canonical.display(),
                        "nsjail binary resolved via additional trusted path"
                    );
                }
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

fn nsjail_supports_flag(binary: &str, flag: &str) -> bool {
    use std::io::Read;

    let mut child = match std::process::Command::new(binary)
        .arg("--help")
        .env_clear()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() >= NSJAIL_HELP_PROBE_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return false,
        }
    }
    let mut text = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        text.push_str(&buf);
    }
    if let Some(mut stderr) = child.stderr.take() {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf);
        text.push_str(&buf);
    }
    text.contains(flag)
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
