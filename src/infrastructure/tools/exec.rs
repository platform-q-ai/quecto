// Shell execution tool: impl Tool for ExecTool.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
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
const DEFAULT_NSJAIL_TMP_SIZE_MB: u64 = 64;
const TRUSTED_NSJAIL_PATHS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/usr/local/bin"];
const EXEC_ENV_ALLOWLIST: &[&str] = &[
    "HOME", "PATH", "LANG", "TZ", "TERM", "SHELL", "USER", "LOGNAME", "TMPDIR",
];

/// System paths to mount read-only inside the nsjail container.
/// These provide the basic toolchain (/bin/sh, common utilities, shared libs).
const NSJAIL_RO_BINDMOUNTS: &[&str] = &["/bin", "/usr", "/lib", "/lib64"];

/// Individual files from `/etc` needed inside the jail.
/// We mount these individually rather than all of `/etc` to avoid exposing
/// host configuration (hostname, machine-id, ssh keys, resolv.conf, etc.).
const NSJAIL_RO_ETC_FILES: &[&str] = &[
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/nsswitch.conf",
    "/etc/passwd",
    "/etc/group",
    "/etc/ssl",
    "/etc/alternatives",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecIsolationMode {
    Native,
    Nsjail,
}

/// nsjail configuration options.
///
/// Resource limits are enforced via rlimits (`--rlimit_as`, `--rlimit_nproc`,
/// `--rlimit_cpu`) which work without root or cgroup access. The cgroup
/// namespace is always disabled (`--disable_clone_newcgroup`).
#[derive(Debug, Clone)]
pub struct NsjailOptions {
    pub binary: String,
    pub network_passthrough: bool,
    /// Virtual address-space limit in MB, enforced via `--rlimit_as`.
    ///
    /// **Note:** This limits *virtual* address space, not physical RSS (unlike
    /// the former `--cgroup_mem_max`). Runtimes that pre-reserve large virtual
    /// regions (Go, JVM) may need a higher value than their actual memory use.
    /// Conversely, `mmap(MAP_NORESERVE)` can bypass this until page faults.
    /// The `--time_limit` wall-clock cap partially mitigates unbounded consumption.
    pub memory_limit_mb: Option<u64>,
    /// Maximum number of processes, enforced via `--rlimit_nproc`.
    ///
    /// **Note:** `RLIMIT_NPROC` is a per-UID limit, not per-jail. Inside
    /// nsjail's user namespace the jailed UID maps to an outer UID, so the
    /// budget is shared with any other processes running as that outer UID.
    /// This is weaker than the former per-cgroup `--cgroup_pids_max` in
    /// multi-tenant scenarios. If concurrent jails are expected, consider
    /// distinct UID mappings per invocation.
    pub pid_limit: Option<u64>,
    /// CPU time limit in seconds, enforced via `--rlimit_cpu`.
    pub cpu_time_limit_secs: Option<u64>,
    /// Wall-clock time limit in seconds, enforced via `--time_limit`.
    pub wall_time_limit_secs: Option<u64>,
    /// Size of the writable tmpfs mounted at `/tmp` inside the jail, in MB.
    ///
    /// Defaults to 64 MB. Set to `None` to disable the `/tmp` tmpfs mount.
    /// Uses the nsjail `-m none:/tmp:tmpfs:size=<bytes>` syntax to ensure
    /// the tmpfs is explicitly bounded (kernel default is 50% of host RAM).
    pub tmp_size_mb: Option<u64>,
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
            tmp_size_mb: Some(DEFAULT_NSJAIL_TMP_SIZE_MB),
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
    /// System paths resolved at construction time to avoid per-execution stat() calls.
    ro_bindmounts: Vec<&'static str>,
    ro_etc_files: Vec<&'static str>,
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
            // Warn if nsjail mode is active but all resource limits are disabled.
            if options.nsjail.memory_limit_mb.is_none()
                && options.nsjail.pid_limit.is_none()
                && options.nsjail.cpu_time_limit_secs.is_none()
                && options.nsjail.wall_time_limit_secs.is_none()
            {
                tracing::warn!(
                    target: "exec",
                    "nsjail isolation is active but all resource limits are disabled. \
                     The jail will run without memory, PID, CPU, or wall-clock limits."
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
        let ro_bindmounts = resolve_ro_bindmounts();
        let ro_etc_files = resolve_ro_etc_files();
        Self {
            workspace,
            sandbox,
            timeout: options.timeout,
            max_capture_bytes: options.max_capture_bytes,
            mode,
            nsjail: options.nsjail,
            ro_bindmounts,
            ro_etc_files,
            startup_warning: warning,
            startup_error,
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
        let ro_bindmounts = resolve_ro_bindmounts();
        let ro_etc_files = resolve_ro_etc_files();
        Self {
            workspace,
            sandbox,
            timeout: options.timeout,
            max_capture_bytes: options.max_capture_bytes,
            mode: options.isolation_mode,
            nsjail: options.nsjail,
            ro_bindmounts,
            ro_etc_files,
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

    /// Returns the nsjail options for inspection in tests/BDD.
    #[cfg(any(test, feature = "test-support"))]
    pub fn nsjail_options(&self) -> &NsjailOptions {
        &self.nsjail
    }

    /// Build the nsjail command for a given workspace and command string.
    /// Exposed for testing to verify argument construction.
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
            ExecIsolationMode::Nsjail => {
                let config = NsjailConfig {
                    options: &self.nsjail,
                    ro_dirs: &self.ro_bindmounts,
                    ro_etc_files: &self.ro_etc_files,
                };
                build_nsjail_command(&self.workspace, command, source_env, &config)
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

/// All nsjail configuration needed to build a command: options + pre-resolved mounts.
struct NsjailConfig<'a> {
    options: &'a NsjailOptions,
    /// Directory-level RO mounts (e.g. /bin, /usr, /lib), resolved at construction.
    ro_dirs: &'a [&'static str],
    /// Individual /etc file RO mounts (e.g. /etc/ld.so.cache), resolved at construction.
    ro_etc_files: &'a [&'static str],
}

fn build_nsjail_command(
    workspace: &Path,
    command: &str,
    source_env: &HashMap<String, String>,
    config: &NsjailConfig<'_>,
) -> tokio::process::Command {
    let options = config.options;
    let mut cmd = tokio::process::Command::new(&options.binary);
    cmd.arg("--quiet")
        .arg("--mode")
        .arg("o")
        .arg("--cwd")
        .arg("/workspace")
        .arg("--bindmount")
        .arg(format!("{}:/workspace", workspace.display()));

    // Mount essential system paths read-only (resolved at construction time).
    for sys_path in config.ro_dirs {
        cmd.arg("--bindmount_ro")
            .arg(format!("{sys_path}:{sys_path}"));
    }
    // Mount individual /etc files needed by the dynamic linker and NSS.
    for etc_path in config.ro_etc_files {
        cmd.arg("--bindmount_ro")
            .arg(format!("{etc_path}:{etc_path}"));
    }

    // Mount a writable tmpfs at /tmp so commands that expect a POSIX-standard
    // writable temp directory (mktemp, compilers, pip, etc.) work out of the box.
    // The tmpfs is ephemeral — automatically cleaned when the jail exits.
    // We use the explicit `-m none:/tmp:tmpfs:size=<bytes>` syntax to bound
    // the tmpfs size (kernel default is 50% of host RAM which is too generous).
    if let Some(tmp_mb) = options.tmp_size_mb {
        let tmp_bytes = tmp_mb * 1024 * 1024;
        cmd.arg("-m")
            .arg(format!("none:/tmp:tmpfs:size={tmp_bytes}"));
    }

    // Disable cgroup namespace — resource limits are enforced via rlimits
    // which work without root or cgroup write access.
    cmd.arg("--disable_clone_newcgroup");

    // Resource limits via rlimits (unprivileged, per-process).
    // --rlimit_as: virtual address space limit in MB.
    // --rlimit_nproc: max processes per UID (effectively per-jail inside
    //   nsjail's user namespace).
    // --rlimit_cpu: CPU time limit in seconds.
    // --time_limit: wall-clock time limit in seconds.
    if let Some(mem) = options.memory_limit_mb {
        cmd.arg("--rlimit_as").arg(mem.to_string());
    }
    if let Some(pid) = options.pid_limit {
        cmd.arg("--rlimit_nproc").arg(pid.to_string());
    }
    if let Some(cpu) = options.cpu_time_limit_secs {
        cmd.arg("--rlimit_cpu").arg(cpu.to_string());
    }
    if let Some(wall) = options.wall_time_limit_secs {
        cmd.arg("--time_limit").arg(wall.to_string());
    }

    if options.network_passthrough {
        cmd.arg("--disable_clone_newnet");
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

    // Ensure temp dir env vars point to the writable tmpfs inside the jail.
    // TMPDIR (POSIX), TMP (common on Linux), TEMP (Python/cross-platform)
    // are all set to /tmp unless the caller explicitly overrides them.
    for var in ["TMPDIR", "TMP", "TEMP"] {
        if !source_env.contains_key(var) {
            cmd.env(var, "/tmp");
        }
    }

    cmd
}

/// Resolve which system RO bindmount paths exist on this host.
/// Called once at construction time to avoid per-execution stat() calls.
fn resolve_ro_bindmounts() -> Vec<&'static str> {
    NSJAIL_RO_BINDMOUNTS
        .iter()
        .copied()
        .filter(|p| Path::new(p).exists())
        .collect()
}

/// Resolve which individual /etc files exist on this host.
/// Called once at construction time.
fn resolve_ro_etc_files() -> Vec<&'static str> {
    NSJAIL_RO_ETC_FILES
        .iter()
        .copied()
        .filter(|p| Path::new(p).exists())
        .collect()
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
