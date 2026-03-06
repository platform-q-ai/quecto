// nsjail isolation: configuration types, command builder, binary validation.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Tokio-side timeout grace period beyond nsjail's wall limit.
/// Set to 0 (no default timeout) — operators configure via config.json.
pub(super) const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(0);
pub(super) const SECRET_ENV_PREFIX: &str = "QUECTO_";
/// Maximum bytes captured per stream before the stream reader stops collecting.
/// Raised to 10 MiB so the tail-truncation window (50 KB) always sees the true
/// tail of the output, not a head-truncated proxy.
pub(super) const MAX_CAPTURE_BYTES: usize = 10 * 1024 * 1024;
pub(super) const STREAM_DRAIN_TIMEOUT_ON_KILL: Duration = Duration::from_millis(250);

/// Default virtual address-space limit (MB) passed to `--rlimit_as`.
///
/// Set to 4 GB because modern runtimes reserve large contiguous virtual
/// regions at startup even when physical RSS is tiny:
///
/// - **Node.js / V8**: reserves ~1–2 GB of virtual space for the heap arena
/// - **JVM**: reserves ~1–4 GB depending on heap flags
/// - **Go**: uses large virtual mappings for goroutine stacks
///
/// `RLIMIT_AS` limits *virtual* address space, not physical RAM.  A Node
/// process using 50 MB of RAM needs 1–2 GB of virtual space to start.
/// 512 MB was too low for all three runtimes.
///
/// Operators running on very constrained hosts can lower this via
/// `tools.exec.nsjail.memory_limit_mb` in their config file.
const DEFAULT_NSJAIL_MEMORY_LIMIT_MB: u64 = 4096;
const DEFAULT_NSJAIL_PID_LIMIT: u64 = 256;
/// CPU time budget: 2 logical cores × 4-hour wall budget = 28 800 CPU-seconds.
///
/// `RLIMIT_CPU` counts accumulated CPU-seconds (user + system) across the
/// entire process tree.  Setting it to 2 × the wall-clock limit means a
/// workload that fully saturates two cores will hit this limit at exactly the
/// same moment as `--time_limit`; single-threaded jobs never hit it before the
/// wall-clock limit does.  Operators on single-core hosts can halve this value.
// CPU time limit removed from defaults — operators configure via config.json.
/// No default wall-clock timeout — operators configure via config.json.
///
/// When 0, nsjail's `--time_limit` is not set, allowing indefinite execution.
/// Exposed as `pub` so `ExecOptions::default()` can derive its Tokio-side
/// timeout from this value.
pub const DEFAULT_NSJAIL_WALL_TIME_LIMIT_SECS: u64 = 0;
/// Writable `/tmp` tmpfs cap: 512 MB (536 870 912 bytes).
///
/// Conservative default that is safe for the project's primary targets
/// (Raspberry Pi, small VPS with 1–2 GB RAM).  tmpfs is RAM-backed, so a
/// runaway process writing to `/tmp` consumes physical RAM; at 512 MB the
/// blast radius is bounded even on a 1 GB host.
///
/// **Multi-tenant note:** each concurrent jail gets its own tmpfs mount, so
/// N simultaneous jails can each consume up to 512 MB of RAM from `/tmp`
/// alone.  Operators on large hosts can raise this via `tools.exec.tmp_size_mb`
/// in their config file.
const DEFAULT_NSJAIL_TMP_SIZE_MB: u64 = 512;

const TRUSTED_NSJAIL_PATHS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/usr/local/bin"];

pub(super) const EXEC_ENV_ALLOWLIST: &[&str] = &[
    "HOME", "PATH", "LANG", "TZ", "TERM", "SHELL", "USER", "LOGNAME", "TMPDIR",
];

/// System paths to mount read-only inside the nsjail container.
const NSJAIL_RO_BINDMOUNTS: &[&str] = &["/bin", "/usr", "/lib", "/lib64"];

/// Individual files from `/etc` needed inside the jail.
///
/// `/etc/resolv.conf` and `/etc/hosts` are required for DNS and hostname
/// resolution. Without them, tools like `curl`, `git`, and `gh` cannot
/// resolve any hostnames even when `network_passthrough` is enabled.
///
/// These files are mounted **unconditionally** (not gated on
/// `network_passthrough`) for two reasons:
/// 1. They are read-only and contain no secrets — only nameserver addresses
///    and hostname/IP mappings that are already accessible to any process
///    on the host with read access to `/etc`.
/// 2. Gating them on `network_passthrough` would require threading that flag
///    through to mount-list construction, adding complexity for negligible
///    security benefit (a network-isolated jail still has `getent hosts`
///    available, so the information would be inferrable anyway).
///
/// `/etc/resolv.conf` is often a symlink (e.g. on systemd hosts it points
/// to `/run/systemd/resolve/stub-resolv.conf`). nsjail's `--bindmount_ro`
/// follows the symlink at mount time, so the real file is mounted correctly
/// at the `/etc/resolv.conf` path inside the jail regardless.
const NSJAIL_RO_ETC_FILES: &[&str] = &[
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/resolv.conf",
    "/etc/hosts",
    "/etc/nsswitch.conf",
    "/etc/passwd",
    "/etc/group",
    "/etc/ssl",
    // Required because /etc/ssl/certs/ca-certificates.crt is a symlink whose
    // target resolves to /etc/ca-certificates/extracted/tls-ca-bundle.pem.
    // Without this mount, TLS libraries (curl, git, gh, openssl) fail with
    // "error adding trust anchors" (exit code 77) even when /etc/ssl is mounted.
    "/etc/ca-certificates",
    "/etc/alternatives",
];

/// Essential `/dev` character devices to bind-mount read-only inside the jail.
///
/// Full `/dev` is intentionally **not** mounted — that would expose block devices
/// (`/dev/sda`, `/dev/mem`, etc.).  Only the safe, universally-needed nodes are
/// included here, resolved at `ExecTool` construction time just like
/// `NSJAIL_RO_ETC_FILES`.
///
/// **`--bindmount_ro` semantics on character devices:** `--bindmount_ro` in
/// nsjail prevents the mount-point itself from being unmounted or re-mounted,
/// but `write(2)` / `read(2)` syscalls to the underlying character device
/// still succeed.  This is the correct behaviour for `/dev/null` (discards
/// writes), `/dev/urandom` (reads entropy), and `/dev/zero` (reads zeros).
///
/// **`/dev/random` note:** On Linux kernels < 5.6 `/dev/random` can block when
/// the entropy pool is depleted.  Since kernel 5.6 it behaves identically to
/// `/dev/urandom`.  Jailed processes needing non-blocking entropy should prefer
/// `/dev/urandom`.
const NSJAIL_RO_DEV_FILES: &[&str] = &["/dev/null", "/dev/urandom", "/dev/random", "/dev/zero"];

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
    /// **This is a virtual address-space cap, not a physical RAM cap.**
    /// `RLIMIT_AS` limits the total `mmap`-able virtual region; RSS (actual
    /// physical memory in use) is not constrained by this setting.
    ///
    /// Modern runtimes (Node.js/V8, JVM, Go) reserve 1–4 GB of virtual
    /// address space at startup even when physical usage is only tens of MB.
    /// The default of 4 GB (`DEFAULT_NSJAIL_MEMORY_LIMIT_MB`) accommodates
    /// all three.  Lower this only on hosts where virtual address space is
    /// genuinely scarce (32-bit environments, very low ulimit).
    pub memory_limit_mb: Option<u64>,
    /// Maximum number of processes, enforced via `--rlimit_nproc`.
    ///
    /// **Note:** `RLIMIT_NPROC` is a per-UID limit shared across all jails
    /// running as the same outer UID.
    pub pid_limit: Option<u64>,
    /// CPU time limit in seconds, enforced via `--rlimit_cpu`.
    ///
    /// Defaults to 28 800 s (2 cores × 4-hour wall budget).  A fully
    /// multi-threaded workload burning two cores exhausts this at the same
    /// moment `--time_limit` fires; single-threaded jobs are always bounded
    /// by `wall_time_limit_secs` first.
    pub cpu_time_limit_secs: Option<u64>,
    /// Wall-clock time limit in seconds, enforced via nsjail's `--time_limit`.
    ///
    /// Defaults to 14 400 s (4 hours).  Catches hung or I/O-blocked processes
    /// that never accumulate enough CPU-seconds to trigger `RLIMIT_CPU`.
    pub wall_time_limit_secs: Option<u64>,
    /// Size of the writable tmpfs at `/tmp` in MB.
    ///
    /// Defaults to 2 048 MB (2 GiB).  Set to `None` to disable the `/tmp`
    /// tmpfs mount.  Uses `-m none:/tmp:tmpfs:size=<bytes>` for explicit
    /// bounding.
    pub tmp_size_mb: Option<u64>,
}

impl Default for NsjailOptions {
    fn default() -> Self {
        Self {
            binary: "nsjail".to_string(),
            network_passthrough: false,
            memory_limit_mb: Some(DEFAULT_NSJAIL_MEMORY_LIMIT_MB),
            pid_limit: Some(DEFAULT_NSJAIL_PID_LIMIT),
            cpu_time_limit_secs: None,
            wall_time_limit_secs: None,
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
    /// Individual /dev character device RO mounts, resolved at construction.
    pub ro_dev_files: &'a [&'static str],
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
    apply_base_nsjail_args(&mut cmd, workspace, config);
    apply_nsjail_resource_limits(&mut cmd, options);
    apply_nsjail_shell_command(&mut cmd, workspace, command);
    apply_nsjail_env(&mut cmd, source_env);
    cmd
}

/// Add basic nsjail flags: quiet mode, working directory, bind mounts.
fn apply_base_nsjail_args(
    cmd: &mut tokio::process::Command,
    workspace: &Path,
    config: &NsjailConfig<'_>,
) {
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
    for dev_path in config.ro_dev_files {
        cmd.arg("--bindmount_ro")
            .arg(format!("{dev_path}:{dev_path}"));
    }

    // Writable tmpfs at /tmp — ephemeral, bounded, POSIX-standard.
    if let Some(tmp_mb) = config.options.tmp_size_mb {
        let tmp_bytes = tmp_mb * 1024 * 1024;
        cmd.arg("-m")
            .arg(format!("none:/tmp:tmpfs:size={tmp_bytes}"));
    }

    cmd.arg("--disable_clone_newcgroup");

    if config.options.network_passthrough {
        cmd.arg("--disable_clone_newnet");
    }
}

/// Add nsjail resource limit flags.
fn apply_nsjail_resource_limits(cmd: &mut tokio::process::Command, options: &NsjailOptions) {
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
}

/// Add the shell command after the `--` separator.
fn apply_nsjail_shell_command(cmd: &mut tokio::process::Command, workspace: &Path, command: &str) {
    cmd.arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(workspace)
        .env_clear();
}

/// Propagate safe environment variables into the nsjail process.
fn apply_nsjail_env(cmd: &mut tokio::process::Command, source_env: &HashMap<String, String>) {
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
}

// ---------------------------------------------------------------------------
// Mount resolution (called once at ExecTool construction time)
// ---------------------------------------------------------------------------

/// Filter a static path list to those that actually exist on the host.
///
/// Called once at [`ExecTool`] construction time so per-invocation path
/// resolution is avoided.
fn resolve_existing(paths: &[&'static str]) -> Vec<&'static str> {
    paths
        .iter()
        .copied()
        .filter(|p| Path::new(p).exists())
        .collect()
}

pub(super) fn resolve_ro_bindmounts() -> Vec<&'static str> {
    resolve_existing(NSJAIL_RO_BINDMOUNTS)
}

pub(super) fn resolve_ro_etc_files() -> Vec<&'static str> {
    resolve_existing(NSJAIL_RO_ETC_FILES)
}

pub(super) fn resolve_ro_dev_files() -> Vec<&'static str> {
    resolve_existing(NSJAIL_RO_DEV_FILES)
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
