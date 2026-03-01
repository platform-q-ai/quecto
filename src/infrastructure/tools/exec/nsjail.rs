// nsjail isolation: configuration types, command builder, binary validation.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub(super) const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const SECRET_ENV_PREFIX: &str = "QUECTO_";
/// Maximum bytes captured per stream before the stream reader stops collecting.
/// Raised to 10 MiB so the tail-truncation window (50 KB) always sees the true
/// tail of the output, not a head-truncated proxy.
pub(super) const MAX_CAPTURE_BYTES: usize = 10 * 1024 * 1024;
pub(super) const STREAM_DRAIN_TIMEOUT_ON_KILL: Duration = Duration::from_millis(250);

const DEFAULT_NSJAIL_MEMORY_LIMIT_MB: u64 = 512;
const DEFAULT_NSJAIL_PID_LIMIT: u64 = 256;
const DEFAULT_NSJAIL_CPU_TIME_LIMIT_SECS: u64 = 30;
const DEFAULT_NSJAIL_WALL_TIME_LIMIT_SECS: u64 = 30;
const DEFAULT_NSJAIL_TMP_SIZE_MB: u64 = 64;

const TRUSTED_NSJAIL_PATHS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/usr/local/bin"];

pub(super) const EXEC_ENV_ALLOWLIST: &[&str] = &[
    "HOME", "PATH", "LANG", "TZ", "TERM", "SHELL", "USER", "LOGNAME", "TMPDIR",
];

/// System paths to mount read-only inside the nsjail container.
const NSJAIL_RO_BINDMOUNTS: &[&str] = &["/bin", "/usr", "/lib", "/lib64"];

/// Individual files from `/etc` needed inside the jail.
const NSJAIL_RO_ETC_FILES: &[&str] = &[
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/nsswitch.conf",
    "/etc/passwd",
    "/etc/group",
    "/etc/ssl",
    "/etc/alternatives",
];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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
    pub memory_limit_mb: Option<u64>,
    /// Maximum number of processes, enforced via `--rlimit_nproc`.
    ///
    /// **Note:** `RLIMIT_NPROC` is a per-UID limit shared across all jails
    /// running as the same outer UID.
    pub pid_limit: Option<u64>,
    /// CPU time limit in seconds, enforced via `--rlimit_cpu`.
    pub cpu_time_limit_secs: Option<u64>,
    /// Wall-clock time limit in seconds, enforced via `--time_limit`.
    pub wall_time_limit_secs: Option<u64>,
    /// Size of the writable tmpfs at `/tmp` in MB.
    ///
    /// Defaults to 64 MB. Set to `None` to disable the `/tmp` tmpfs mount.
    /// Uses `-m none:/tmp:tmpfs:size=<bytes>` for explicit bounding.
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

/// Bundle of nsjail config needed to build a command: options + pre-resolved mounts.
pub(super) struct NsjailConfig<'a> {
    pub options: &'a NsjailOptions,
    /// Directory-level RO mounts (e.g. /bin, /usr, /lib), resolved at construction.
    pub ro_dirs: &'a [&'static str],
    /// Individual /etc file RO mounts, resolved at construction.
    pub ro_etc_files: &'a [&'static str],
}

// ---------------------------------------------------------------------------
// Command builder
// ---------------------------------------------------------------------------

pub(super) fn build_nsjail_command(
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

    for sys_path in config.ro_dirs {
        cmd.arg("--bindmount_ro")
            .arg(format!("{sys_path}:{sys_path}"));
    }
    for etc_path in config.ro_etc_files {
        cmd.arg("--bindmount_ro")
            .arg(format!("{etc_path}:{etc_path}"));
    }

    // Writable tmpfs at /tmp — ephemeral, bounded, POSIX-standard.
    if let Some(tmp_mb) = options.tmp_size_mb {
        let tmp_bytes = tmp_mb * 1024 * 1024;
        cmd.arg("-m")
            .arg(format!("none:/tmp:tmpfs:size={tmp_bytes}"));
    }

    cmd.arg("--disable_clone_newcgroup");

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

    // TMPDIR (POSIX), TMP (common on Linux), TEMP (Python/cross-platform)
    for var in ["TMPDIR", "TMP", "TEMP"] {
        if !source_env.contains_key(var) {
            cmd.env(var, "/tmp");
        }
    }

    cmd
}

// ---------------------------------------------------------------------------
// Mount resolution (called once at ExecTool construction time)
// ---------------------------------------------------------------------------

pub(super) fn resolve_ro_bindmounts() -> Vec<&'static str> {
    NSJAIL_RO_BINDMOUNTS
        .iter()
        .copied()
        .filter(|p| Path::new(p).exists())
        .collect()
}

pub(super) fn resolve_ro_etc_files() -> Vec<&'static str> {
    NSJAIL_RO_ETC_FILES
        .iter()
        .copied()
        .filter(|p| Path::new(p).exists())
        .collect()
}

// ---------------------------------------------------------------------------
// Binary resolution & security checks
// ---------------------------------------------------------------------------

pub(super) fn resolve_nsjail_binary(binary: &str) -> Option<String> {
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

// ---------------------------------------------------------------------------
// PathBuf wrapper used in ExecTool construction
// ---------------------------------------------------------------------------
