//! Environment registry records, targets, and lifecycle values.

use std::path::PathBuf;

/// Lifecycle status of one committed environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentStatus {
    /// Live and joinable.
    Running,
    /// A kill claim is outstanding; not joinable, not yet stopped.
    Killing,
    /// Kill succeeded; terminal. The record stays listed, the ref is never reused.
    Stopped,
    /// Kill failed; retryable via another kill, with `last_error` retained.
    CleanupFailed,
    /// Emptied after its swarm run ended or lost its coordinator (#1924): the
    /// final-member kill was deliberately withheld so the board, checkout and
    /// unpushed work survive for inspection. Killable by explicit control.
    Retained,
    /// Runtime processes are gone, while the workspace and environment state
    /// are deliberately preserved for data recovery. This state is terminal
    /// for execution and therefore never joinable; only an explicit
    /// `kill_container` discards its preserved data.
    Preserved,
}

/// How a caller addresses an existing environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentTarget {
    /// Session-scoped `CN` ref minted by this registry.
    Ref(String),
    /// Optional user-facing environment name; must resolve unambiguously.
    Name(String),
}

/// Resolution failures. Never guesses: unknown, ambiguous, stopped, and stale
/// targets each fail with their own actionable error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentLookupError {
    Unknown(String),
    Ambiguous(String),
    Stopped(String),
    Stale(String),
    /// The durable registry could not be read at startup (round 2 F-B,
    /// #2033): a target this session did not create is not unknown, it is
    /// unknowable, and the store's own account says why.
    Unreadable(String),
}

impl std::fmt::Display for EnvironmentLookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(t) => write!(f, "environment '{t}' is unknown in this session"),
            Self::Ambiguous(t) => write!(f, "environment name '{t}' is ambiguous in this session"),
            Self::Stopped(t) => write!(f, "environment '{t}' is stopped"),
            Self::Stale(t) => write!(
                f,
                "environment '{t}' is stale: cleanup is pending or failed; retry kill_container"
            ),
            Self::Unreadable(error) => write!(f, "registry unreadable: {error}"),
        }
    }
}

impl std::error::Error for EnvironmentLookupError {}

/// One committed script-managed environment known to this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentRecord {
    /// Session-local, never-reused ref (e.g. `C1`) minted by this registry.
    pub environment_ref: String,
    /// Script/runtime-owned environment identity from the create result.
    pub environment_id: String,
    /// Hidden environment UUID minted by Quecto; distinct from the ref, the
    /// runtime id, and every member agent UUID.
    pub environment_uuid: String,
    /// Optional user-facing environment name.
    pub name: Option<String>,
    /// Workspace path reported by the create result; shared by all members.
    pub workspace_path: PathBuf,
    /// Repository URL the environment was created for.
    pub repository: String,
    /// Name of the configured container script set that created it.
    pub script_name: String,
    /// Exec argv retained at create time; joins use this even if the
    /// configured default script set changes later.
    pub retained_exec_argv: Vec<String>,
    /// Kill argv retained at create time; final-member and explicit cleanup
    /// use this exactly once per successful kill.
    pub retained_kill_argv: Vec<String>,
    /// Cleanup argv retained at create time. Runs instead of `kill` when a
    /// launch fails after creation, and serves as the final-member teardown
    /// fallback for script sets without a configured `kill` — retained on the
    /// record so it survives the creator exiting before other members.
    pub retained_cleanup_argv: Vec<String>,
    /// Inspect argv retained at create time. Runs exactly once per dead
    /// member post-mortem; retained on the record (surviving zero members and
    /// failed inspects) so it stays available for retry (#1369 slice 3).
    pub retained_inspect_argv: Vec<String>,
    /// Member agent UUIDs, in join order.
    pub members: Vec<String>,
    pub status: EnvironmentStatus,
    /// Metadata object from the create result.
    pub metadata: serde_json::Value,
    /// Last cleanup error, retained while status is `CleanupFailed`.
    pub last_error: Option<String>,
    /// Where the record came from (#2024 S4d): created by this session, or
    /// restored from the durable registry another (or an earlier) session
    /// wrote.
    pub origin: EnvironmentOrigin,
    /// Key of the session that created the environment (empty for a
    /// session-less run).
    pub created_by: String,
    /// Creation time as seconds since the Unix epoch, when the creator
    /// recorded one.
    pub created_at: Option<u64>,
}

/// Whether this session created the environment or inherited it from the
/// durable registry (#2024 S4d).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvironmentOrigin {
    /// Committed by this session's own create: this session owns its
    /// final-member teardown.
    #[default]
    Created,
    /// Seeded at startup from the durable registry: members of the creating
    /// session are unknown here, and a joiner leaving it never triggers the
    /// final-member kill — only an explicit kill ends it.
    Restored,
}

impl EnvironmentRecord {
    /// User-facing status label: a live environment with no members reads as
    /// `empty`, otherwise the lifecycle status names itself.
    pub fn status_label(&self) -> &'static str {
        match self.status {
            EnvironmentStatus::Running if self.members.is_empty() => "empty",
            EnvironmentStatus::Running => "running",
            EnvironmentStatus::Killing => "killing",
            EnvironmentStatus::Stopped => "stopped",
            EnvironmentStatus::CleanupFailed => "cleanup-failed",
            EnvironmentStatus::Retained => "retained",
            EnvironmentStatus::Preserved => "preserved",
        }
    }

    /// Mark the record retained (#1924): `Retained`, no members, `reason`
    /// under `metadata.retained` (the one key that says why a box is kept).
    pub fn retain_with(&mut self, reason: &str) {
        self.status = EnvironmentStatus::Retained;
        self.members.clear();
        if let Some(object) = self.metadata.as_object_mut() {
            object.insert("retained".to_string(), serde_json::json!(reason));
        } else {
            self.metadata = serde_json::json!({ "retained": reason });
        }
    }
}

/// Mint the hidden environment UUID committed with each new environment.
/// Distinct from the `CN` ref, the runtime id, and agent UUIDs by
/// construction (fresh v4 UUID per environment).
pub fn mint_environment_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
