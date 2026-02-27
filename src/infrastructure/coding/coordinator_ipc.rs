//! File-based IPC for coordinator subagent communication.
//!
//! The main agent writes command JSON files to `coordinator/inbox/` and
//! reads response files from `coordinator/outbox/`. The coordinator writes
//! proactive notifications to `coordinator/notifications/` and periodic
//! state snapshots to `coordinator/state.json`.
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
use std::path::PathBuf;

use crate::domain::coding_ipc::{
    CoordinatorIpc, CoordinatorIpcCommand, CoordinatorIpcResponse, CoordinatorNotification,
    CoordinatorState,
};

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
    /// are created eagerly.
    pub fn new(base: impl Into<PathBuf>) -> Result<Self, String> {
        let base = base.into();
        for sub in &["inbox", "outbox", "notifications"] {
            fs::create_dir_all(base.join(sub)).map_err(|e| format!("mkdir {sub}: {e}"))?;
        }
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
    /// Uses `kill -0 <pid>` which sends signal 0 (existence check only).
    /// Exit code 0 means the process exists; non-zero means it doesn't
    /// (or we lack permission, but that still implies it exists).
    fn process_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        // `kill -0` checks existence without actually sending a signal.
        // Returns exit code 0 if the process exists.
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

impl CoordinatorIpc for FileCoordinatorIpc {
    fn write_command(&self, cmd: &CoordinatorIpcCommand) -> Result<(), String> {
        let path = self.inbox_dir().join(format!("{}.json", cmd.command_id));
        let json = serde_json::to_string_pretty(cmd).map_err(|e| format!("serialize: {e}"))?;
        fs::write(&path, json).map_err(|e| format!("write inbox: {e}"))
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
            let content = fs::read_to_string(&path).map_err(|e| format!("read {path:?}: {e}"))?;
            let cmd: CoordinatorIpcCommand =
                serde_json::from_str(&content).map_err(|e| format!("parse {path:?}: {e}"))?;
            commands.push(cmd);
        }
        // Sort by command_id for deterministic ordering.
        commands.sort_by(|a, b| a.command_id.cmp(&b.command_id));
        Ok(commands)
    }

    fn acknowledge_command(&self, command_id: &str) -> Result<(), String> {
        let path = self.inbox_dir().join(format!("{command_id}.json"));
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("remove inbox {command_id}: {e}"))?;
        }
        Ok(())
    }

    fn write_response(&self, resp: &CoordinatorIpcResponse) -> Result<(), String> {
        let path = self.outbox_dir().join(format!("{}.json", resp.command_id));
        let json = serde_json::to_string_pretty(resp).map_err(|e| format!("serialize: {e}"))?;
        fs::write(&path, json).map_err(|e| format!("write outbox: {e}"))
    }

    fn read_response(&self, command_id: &str) -> Result<Option<CoordinatorIpcResponse>, String> {
        let path = self.outbox_dir().join(format!("{command_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|e| format!("read outbox: {e}"))?;
        let resp: CoordinatorIpcResponse =
            serde_json::from_str(&content).map_err(|e| format!("parse outbox: {e}"))?;
        // Remove the response file after reading.
        let _ = fs::remove_file(&path);
        Ok(Some(resp))
    }

    fn write_notification(&self, notif: &CoordinatorNotification) -> Result<(), String> {
        // Filename: <ts>_<type>.json (ts sanitized for filesystem safety)
        let ts_safe = notif.ts.replace(':', "-").replace(' ', "_");
        let filename = format!("{}_{}.json", ts_safe, notif.notification_type);
        let path = self.notifications_dir().join(filename);
        let json = serde_json::to_string_pretty(notif).map_err(|e| format!("serialize: {e}"))?;
        fs::write(&path, json).map_err(|e| format!("write notification: {e}"))
    }

    fn read_notifications(&self) -> Result<Vec<CoordinatorNotification>, String> {
        let dir = self.notifications_dir();
        let mut notifications = Vec::new();
        let entries = fs::read_dir(&dir).map_err(|e| format!("read notifications dir: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read_to_string(&path).map_err(|e| format!("read {path:?}: {e}"))?;
            let notif: CoordinatorNotification =
                serde_json::from_str(&content).map_err(|e| format!("parse {path:?}: {e}"))?;
            notifications.push(notif);
        }
        // Sort by timestamp for deterministic ordering.
        notifications.sort_by(|a, b| a.ts.cmp(&b.ts));
        Ok(notifications)
    }

    fn acknowledge_notification(&self, filename: &str) -> Result<(), String> {
        let path = self.notifications_dir().join(filename);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("remove notification: {e}"))?;
        }
        Ok(())
    }

    fn write_state(&self, state: &CoordinatorState) -> Result<(), String> {
        let json = serde_json::to_string_pretty(state).map_err(|e| format!("serialize: {e}"))?;
        fs::write(self.state_path(), json).map_err(|e| format!("write state: {e}"))
    }

    fn read_state(&self) -> Result<Option<CoordinatorState>, String> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).map_err(|e| format!("read state: {e}"))?;
        let state: CoordinatorState =
            serde_json::from_str(&content).map_err(|e| format!("parse state: {e}"))?;
        Ok(Some(state))
    }

    fn write_pid(&self, pid: u32) -> Result<(), String> {
        fs::write(self.pid_path(), pid.to_string()).map_err(|e| format!("write pid: {e}"))
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
}
