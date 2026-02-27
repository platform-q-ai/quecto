//! File-based IPC for coordinator subagent communication.
//!
//! The main agent writes command JSON files to `coordinator/inbox/` and
//! reads response files from `coordinator/outbox/`. The coordinator writes
//! proactive notifications to `coordinator/notifications/` and periodic
//! state snapshots to `coordinator/state.json`.
//!
//! Security hardening:
//! - All filenames are validated (alphanumeric + hyphens/underscores only)
//! - File reads are capped at 1 MiB to prevent OOM
//! - Writes use atomic rename (write to `.tmp` then rename)
//! - Directories are created with 0700 permissions on Unix
//!
//! Layout:
//! ```text
//! <base_dir>/coordinator/
//! ├── inbox/              # Main agent writes, coordinator reads
//! ├── outbox/             # Coordinator writes, main agent reads
//! ├── notifications/      # Coordinator writes proactively
//! ├── state.json          # Periodic coordinator snapshot
//! └── pid                 # Coordinator process PID
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::coding_ipc::{
    CoordinatorIpc, CoordinatorIpcCommand, CoordinatorIpcResponse, CoordinatorNotification,
    CoordinatorState,
};

/// Maximum IPC file size (1 MiB). Files larger than this are rejected
/// to prevent OOM from malicious or corrupted files.
const MAX_IPC_FILE_SIZE: u64 = 1024 * 1024;

/// Maximum number of notifications returned per `read_notifications()` call.
const MAX_NOTIFICATIONS_PER_READ: usize = 100;

/// File-based implementation of `CoordinatorIpc`.
///
/// All operations are synchronous filesystem I/O. The directory structure
/// is created on first use if it doesn't exist.
#[derive(Debug, Clone)]
pub struct FileCoordinatorIpc {
    /// Root directory: `<base_dir>/coordinator/`
    base: PathBuf,
}

impl FileCoordinatorIpc {
    /// Create a new `FileCoordinatorIpc` rooted at the given directory.
    ///
    /// The directory and its subdirectories (inbox, outbox, notifications)
    /// are created eagerly with restricted permissions (0700 on Unix).
    pub fn new(base: impl Into<PathBuf>) -> Result<Self, String> {
        let base = base.into();
        for sub in &["inbox", "outbox", "notifications"] {
            let dir = base.join(sub);
            fs::create_dir_all(&dir).map_err(|e| format!("mkdir {sub}: {e}"))?;
            set_dir_permissions(&dir);
        }
        // Also restrict the base directory itself.
        set_dir_permissions(&base);
        Ok(Self { base })
    }

    fn inbox_dir(&self) -> PathBuf {
        self.base.join("inbox")
    }

    fn outbox_dir(&self) -> PathBuf {
        self.base.join("outbox")
    }

    fn notifications_dir(&self) -> PathBuf {
        self.base.join("notifications")
    }

    fn state_path(&self) -> PathBuf {
        self.base.join("state.json")
    }

    fn pid_path(&self) -> PathBuf {
        self.base.join("pid")
    }

    /// Check whether an OS process with the given PID is running.
    ///
    /// Uses the `kill(pid, 0)` syscall pattern: signal 0 checks existence
    /// without actually sending a signal. Returns true if the process
    /// exists, false otherwise.
    fn process_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        // Use /proc/<pid> existence as a fast, portable liveness check.
        // This avoids shelling out to `kill` and avoids needing libc FFI.
        Path::new(&format!("/proc/{pid}")).exists()
    }
}

/// Validate that a filename component contains only safe characters.
///
/// Allowed: alphanumeric, hyphens, underscores, dots.
/// Rejects: `/`, `\`, `..`, null bytes, and any other path-unsafe chars.
fn validate_ipc_filename(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty filename".to_string());
    }
    if name.contains("..") {
        return Err(format!("path traversal in filename: {name}"));
    }
    if name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        Ok(())
    } else {
        Err(format!("unsafe characters in filename: {name}"))
    }
}

/// Read a file with a size cap to prevent OOM on malicious files.
fn read_file_capped(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| format!("metadata {path:?}: {e}"))?;
    if meta.len() > MAX_IPC_FILE_SIZE {
        return Err(format!(
            "file too large ({} bytes, max {}): {path:?}",
            meta.len(),
            MAX_IPC_FILE_SIZE
        ));
    }
    fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))
}

/// Atomic write: write to a `.tmp` sibling, then rename into place.
///
/// This prevents readers from seeing partial/torn JSON files.
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| format!("write tmp {tmp:?}: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))
}

/// Set 0700 permissions on a directory (Unix only, no-op elsewhere).
#[cfg(unix)]
fn set_dir_permissions(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_permissions(_dir: &Path) {}

/// Sanitize a notification timestamp for use in filenames.
///
/// Replaces colons and spaces with safe characters, and rejects
/// any characters that could cause path traversal.
fn sanitize_notification_ts(ts: &str) -> Result<String, String> {
    let safe = ts.replace(':', "-").replace(' ', "_");
    if safe.contains('/') || safe.contains('\\') || safe.contains("..") || safe.contains('\0') {
        return Err(format!("unsafe timestamp for filename: {ts}"));
    }
    Ok(safe)
}

impl CoordinatorIpc for FileCoordinatorIpc {
    fn write_command(&self, cmd: &CoordinatorIpcCommand) -> Result<(), String> {
        validate_ipc_filename(&cmd.command_id)?;
        let path = self.inbox_dir().join(format!("{}.json", cmd.command_id));
        let json = serde_json::to_string_pretty(cmd).map_err(|e| format!("serialize: {e}"))?;
        atomic_write(&path, &json)
    }

    fn read_pending_commands(&self) -> Result<Vec<CoordinatorIpcCommand>, String> {
        let dir = self.inbox_dir();
        let mut commands = Vec::new();
        let entries = fs::read_dir(&dir).map_err(|e| format!("read inbox dir: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = read_file_capped(&path)?;
            let cmd: CoordinatorIpcCommand =
                serde_json::from_str(&content).map_err(|e| format!("parse {path:?}: {e}"))?;
            commands.push(cmd);
        }
        // Sort by command_id for deterministic ordering.
        commands.sort_by(|a, b| a.command_id.cmp(&b.command_id));
        Ok(commands)
    }

    fn acknowledge_command(&self, command_id: &str) -> Result<(), String> {
        validate_ipc_filename(command_id)?;
        let path = self.inbox_dir().join(format!("{command_id}.json"));
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("remove inbox {command_id}: {e}"))?;
        }
        Ok(())
    }

    fn write_response(&self, resp: &CoordinatorIpcResponse) -> Result<(), String> {
        validate_ipc_filename(&resp.command_id)?;
        let path = self.outbox_dir().join(format!("{}.json", resp.command_id));
        let json = serde_json::to_string_pretty(resp).map_err(|e| format!("serialize: {e}"))?;
        atomic_write(&path, &json)
    }

    fn read_response(&self, command_id: &str) -> Result<Option<CoordinatorIpcResponse>, String> {
        validate_ipc_filename(command_id)?;
        let path = self.outbox_dir().join(format!("{command_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        let content = read_file_capped(&path)?;
        // Parse before deleting to avoid losing the response on parse failure.
        let resp: CoordinatorIpcResponse =
            serde_json::from_str(&content).map_err(|e| format!("parse outbox: {e}"))?;
        // Remove the response file after successful parse.
        let _ = fs::remove_file(&path);
        Ok(Some(resp))
    }

    fn write_notification(&self, notif: &CoordinatorNotification) -> Result<(), String> {
        // Validate ts before building filename.
        sanitize_notification_ts(&notif.ts)?;
        let filename = crate::domain::coding_ipc::notification_filename(notif);
        validate_ipc_filename(&filename)?;
        let path = self.notifications_dir().join(filename);
        let json = serde_json::to_string_pretty(notif).map_err(|e| format!("serialize: {e}"))?;
        atomic_write(&path, &json)
    }

    fn read_notifications(&self) -> Result<Vec<CoordinatorNotification>, String> {
        let dir = self.notifications_dir();
        let mut notifications = Vec::new();
        let entries = fs::read_dir(&dir).map_err(|e| format!("read notifications dir: {e}"))?;
        for entry in entries {
            if notifications.len() >= MAX_NOTIFICATIONS_PER_READ {
                break;
            }
            let entry = entry.map_err(|e| format!("read entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match read_file_capped(&path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(notif) => notifications.push(notif),
                    Err(e) => {
                        tracing::warn!("skip malformed notification {path:?}: {e}");
                    }
                },
                Err(e) => {
                    tracing::warn!("skip unreadable notification {path:?}: {e}");
                }
            }
        }
        // Sort by timestamp for deterministic ordering.
        notifications.sort_by(|a: &CoordinatorNotification, b| a.ts.cmp(&b.ts));
        Ok(notifications)
    }

    fn acknowledge_notification(&self, filename: &str) -> Result<(), String> {
        validate_ipc_filename(filename)?;
        let path = self.notifications_dir().join(filename);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("remove notification: {e}"))?;
        }
        Ok(())
    }

    fn write_state(&self, state: &CoordinatorState) -> Result<(), String> {
        let json = serde_json::to_string_pretty(state).map_err(|e| format!("serialize: {e}"))?;
        atomic_write(&self.state_path(), &json)
    }

    fn read_state(&self) -> Result<Option<CoordinatorState>, String> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = read_file_capped(&path)?;
        let state: CoordinatorState =
            serde_json::from_str(&content).map_err(|e| format!("parse state: {e}"))?;
        Ok(Some(state))
    }

    fn write_pid(&self, pid: u32) -> Result<(), String> {
        atomic_write(&self.pid_path(), &pid.to_string())
    }

    fn read_pid(&self) -> Result<Option<u32>, String> {
        let path = self.pid_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|e| format!("read pid: {e}"))?;
        let pid: u32 = content
            .trim()
            .parse()
            .map_err(|e| format!("parse pid: {e}"))?;
        Ok(Some(pid))
    }

    fn is_coordinator_alive(&self) -> bool {
        match self.read_pid() {
            Ok(Some(pid)) => Self::process_alive(pid),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ipc() -> (TempDir, FileCoordinatorIpc) {
        let td = TempDir::new().unwrap();
        let ipc = FileCoordinatorIpc::new(td.path().join("coordinator")).unwrap();
        (td, ipc)
    }

    #[test]
    fn test_write_and_read_command() {
        let (_td, ipc) = make_ipc();
        let cmd = CoordinatorIpcCommand {
            command_id: "cmd_001".to_string(),
            action: "run".to_string(),
            payload: serde_json::json!({"goal": "Fix"}),
        };
        ipc.write_command(&cmd).unwrap();
        let cmds = ipc.read_pending_commands().unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].command_id, "cmd_001");
        assert_eq!(cmds[0].action, "run");
    }

    #[test]
    fn test_acknowledge_command() {
        let (_td, ipc) = make_ipc();
        let cmd = CoordinatorIpcCommand {
            command_id: "cmd_ack".to_string(),
            action: "status".to_string(),
            payload: serde_json::Value::Null,
        };
        ipc.write_command(&cmd).unwrap();
        assert_eq!(ipc.read_pending_commands().unwrap().len(), 1);
        ipc.acknowledge_command("cmd_ack").unwrap();
        assert_eq!(ipc.read_pending_commands().unwrap().len(), 0);
    }

    #[test]
    fn test_write_and_read_response() {
        let (_td, ipc) = make_ipc();
        let resp = CoordinatorIpcResponse {
            command_id: "cmd_resp".to_string(),
            ok: true,
            body: Some(serde_json::json!({"job_id": "j1"})),
            error: None,
        };
        ipc.write_response(&resp).unwrap();
        let read = ipc.read_response("cmd_resp").unwrap();
        assert!(read.is_some());
        let r = read.unwrap();
        assert!(r.ok);
        assert_eq!(r.command_id, "cmd_resp");
    }

    #[test]
    fn test_read_response_missing() {
        let (_td, ipc) = make_ipc();
        let read = ipc.read_response("nonexistent").unwrap();
        assert!(read.is_none());
    }

    #[test]
    fn test_write_and_read_notification() {
        let (_td, ipc) = make_ipc();
        use crate::domain::coding_ipc::NotificationType;
        let notif = CoordinatorNotification {
            notification_type: NotificationType::JobFailed,
            job_id: Some("j1".to_string()),
            job_ids: vec![],
            detail: Some("crashed".to_string()),
            no_progress_minutes: None,
            ts: "2026-01-15T10:00:00Z".to_string(),
        };
        ipc.write_notification(&notif).unwrap();
        let notifs = ipc.read_notifications().unwrap();
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].notification_type, NotificationType::JobFailed);
    }

    #[test]
    fn test_notifications_ordered_by_timestamp() {
        let (_td, ipc) = make_ipc();
        use crate::domain::coding_ipc::NotificationType;
        for (i, ts) in [
            "2026-01-15T10:03:00Z",
            "2026-01-15T10:01:00Z",
            "2026-01-15T10:02:00Z",
        ]
        .iter()
        .enumerate()
        {
            let notif = CoordinatorNotification {
                notification_type: NotificationType::JobFailed,
                job_id: Some(format!("j{i}")),
                job_ids: vec![],
                detail: None,
                no_progress_minutes: None,
                ts: ts.to_string(),
            };
            ipc.write_notification(&notif).unwrap();
        }
        let notifs = ipc.read_notifications().unwrap();
        assert_eq!(notifs.len(), 3);
        assert!(notifs[0].ts <= notifs[1].ts);
        assert!(notifs[1].ts <= notifs[2].ts);
    }

    #[test]
    fn test_acknowledge_notification() {
        let (_td, ipc) = make_ipc();
        use crate::domain::coding_ipc::NotificationType;
        let notif = CoordinatorNotification {
            notification_type: NotificationType::WorkerBlocked,
            job_id: Some("j1".to_string()),
            job_ids: vec![],
            detail: Some("question".to_string()),
            no_progress_minutes: None,
            ts: "2026-01-15T10:00:00Z".to_string(),
        };
        ipc.write_notification(&notif).unwrap();
        // Find the filename
        let dir = ipc.notifications_dir();
        let files: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files.len(), 1);
        ipc.acknowledge_notification(&files[0]).unwrap();
        assert_eq!(ipc.read_notifications().unwrap().len(), 0);
    }

    #[test]
    fn test_write_and_read_state() {
        let (_td, ipc) = make_ipc();
        let state = CoordinatorState {
            alive: true,
            active_jobs: 3,
            last_heartbeat: "2026-01-15T10:00:00Z".to_string(),
            job_summary: serde_json::json!({"running": 2, "queued": 1}),
        };
        ipc.write_state(&state).unwrap();
        let read = ipc.read_state().unwrap().unwrap();
        assert!(read.alive);
        assert_eq!(read.active_jobs, 3);
    }

    #[test]
    fn test_read_state_missing() {
        let (_td, ipc) = make_ipc();
        assert!(ipc.read_state().unwrap().is_none());
    }

    #[test]
    fn test_write_and_read_pid() {
        let (_td, ipc) = make_ipc();
        ipc.write_pid(12345).unwrap();
        assert_eq!(ipc.read_pid().unwrap(), Some(12345));
    }

    #[test]
    fn test_read_pid_missing() {
        let (_td, ipc) = make_ipc();
        assert_eq!(ipc.read_pid().unwrap(), None);
    }

    #[test]
    fn test_is_coordinator_alive_with_current_pid() {
        let (_td, ipc) = make_ipc();
        let my_pid = std::process::id();
        ipc.write_pid(my_pid).unwrap();
        assert!(ipc.is_coordinator_alive());
    }

    #[test]
    fn test_is_coordinator_alive_with_dead_pid() {
        let (_td, ipc) = make_ipc();
        // Use a very high PID unlikely to exist.
        ipc.write_pid(4_000_000).unwrap();
        assert!(!ipc.is_coordinator_alive());
    }

    #[test]
    fn test_is_coordinator_alive_no_pid_file() {
        let (_td, ipc) = make_ipc();
        assert!(!ipc.is_coordinator_alive());
    }

    #[test]
    fn test_path_traversal_rejected_in_command_id() {
        let (_td, ipc) = make_ipc();
        let cmd = CoordinatorIpcCommand {
            command_id: "../../etc/passwd".to_string(),
            action: "run".to_string(),
            payload: serde_json::Value::Null,
        };
        assert!(ipc.write_command(&cmd).is_err());
    }

    #[test]
    fn test_path_traversal_rejected_in_acknowledge() {
        let (_td, ipc) = make_ipc();
        assert!(ipc.acknowledge_notification("../../etc/passwd").is_err());
    }

    #[test]
    fn test_oversized_file_rejected() {
        let (_td, ipc) = make_ipc();
        // Write a file larger than MAX_IPC_FILE_SIZE directly
        let path = ipc.inbox_dir().join("big.json");
        let big_content = "x".repeat((MAX_IPC_FILE_SIZE + 1) as usize);
        fs::write(&path, big_content).unwrap();
        assert!(ipc.read_pending_commands().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_directory_permissions_are_restricted() {
        use std::os::unix::fs::PermissionsExt;
        let (_td, ipc) = make_ipc();
        let mode = fs::metadata(&ipc.base).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "base dir should be 0700, got {mode:o}");
    }
}
