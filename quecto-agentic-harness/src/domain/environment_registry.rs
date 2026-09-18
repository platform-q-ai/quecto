//! Session-scoped registry of script-managed environments.
//!
//! Per ADR-0021 composition builds exactly one registry per session and
//! injects it into the launch services. It is the authority for minting
//! never-reused `C1`-style environment refs and for recording which
//! environments this session has committed: hidden environment UUID, optional
//! name, script/runtime identity, retained script argv, member agent UUIDs,
//! status, metadata, and last error (#1369 slice 2).
//!
//! Since #2024 S4d the registry is durable: every transition is observed by
//! an optional [`EnvironmentJournal`] the application installs over its
//! store, refs are allocated through it (unique across every session of one
//! base directory), and a harness restart seeds the registry with the
//! records of earlier sessions as *restored* records — reachable for a
//! join, a listing or a kill, but never torn down by a joiner's exit.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    /// unpushed work survive for inspection. Killable only by an explicit
    /// `kill_container`; a join is admitted for inspection but never revives
    /// it (no automatic teardown can follow), so a rolled-back or exited
    /// joiner leaves it retained.
    Retained,
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
        }
    }
}

/// Mint the hidden environment UUID committed with each new environment.
/// Distinct from the `CN` ref, the runtime id, and agent UUIDs by
/// construction (fresh v4 UUID per environment).
pub fn mint_environment_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// What a journal write came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalWrite {
    /// The record is on file as given.
    Written,
    /// The record on file no longer had the status the write expected
    /// (another session moved it on); nothing was written and `current`
    /// is what stands.
    Superseded { current: EnvironmentStatus },
    /// The journal could not be written; the account is the journal's own.
    Unavailable,
}

/// Why a ref could not be minted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefAllocationError {
    /// A durable registry's journal could not allocate: the base
    /// directory's registry is unreadable or unwritable, so a ref minted
    /// from memory could collide with one a live session holds there.
    #[error("environment ref could not be allocated from the durable registry: {0}")]
    JournalUnavailable(String),
}

/// The durable side of the registry (#2024 S4d): where refs are allocated
/// and where every committed record and transition is written. Installed
/// by the application over its store; the registry only reports — it never
/// reads the journal back (restore is the application's, at startup).
#[derive(Clone)]
pub struct EnvironmentJournal {
    /// Allocate the next ref number, unique across every session sharing the
    /// base directory. `Err` means the journal could not allocate; the
    /// registry then refuses to mint (review F9, #2033) — a counter minted
    /// from memory could collide with a ref a live session holds.
    pub allocate_ref: Arc<dyn Fn() -> Result<u64, String> + Send + Sync>,
    /// A record was committed or one of its persisted fields changed. With
    /// `expected` the write is compare-and-set: applied only while the
    /// record on file still has that status (review F5, #2033 — a record
    /// another session created is written conditionally, never replaced
    /// whole, so a joiner's inspect cannot revert its creator's `retained`).
    pub recorded:
        Arc<dyn Fn(&EnvironmentRecord, Option<&EnvironmentStatus>) -> JournalWrite + Send + Sync>,
    /// A record was removed (a rolled-back create).
    pub forgotten: Arc<dyn Fn(&str) + Send + Sync>,
}

impl std::fmt::Debug for EnvironmentJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentJournal").finish_non_exhaustive()
    }
}

/// Proof that the caller holds the exclusive right to run this environment's
/// kill operation. Only `begin_kill`/`remove_member` hand one out; it must be
/// settled with `complete_kill` or `fail_kill`.
#[derive(Debug)]
pub struct KillClaim {
    environment_ref: String,
}

/// Proof that the caller holds the exclusive right to run this environment's
/// retained inspect for one dead member (#1369 slice 3). Only `begin_inspect`
/// hands one out — at most once per (environment, member) — and it must be
/// settled with `record_inspect_success` or `record_inspect_failure`.
#[derive(Debug)]
pub struct InspectClaim {
    environment_ref: String,
}

#[derive(Debug, Default)]
struct EnvironmentRegistryState {
    next_ref: u64,
    entries: BTreeMap<String, EnvironmentRecord>,
    /// (environment_ref, member agent UUID) pairs whose post-mortem inspect
    /// has been claimed. Repeated EOF/reset death signals for the same member
    /// find their pair already present and claim nothing.
    inspect_claims: std::collections::BTreeSet<(String, String)>,
    /// Environments carrying a recorded inspect failure. A later successful
    /// kill must not erase that truthfully persisted error.
    inspect_failures: std::collections::BTreeSet<String>,
    /// For every restored record, the status the journal is known to hold:
    /// what was loaded, then what this registry last wrote (or learnt it
    /// was superseded by). The expectation each journal write is made on.
    journalled: BTreeMap<String, EnvironmentStatus>,
}

/// Cloneable handle to one session's environment registry.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentRegistry {
    state: Arc<Mutex<EnvironmentRegistryState>>,
    journal: Option<EnvironmentJournal>,
    /// Serialises snapshot-and-write so two transitions of one record
    /// reach the journal in the order they happened: the snapshot is
    /// taken and written under this lock (never under the state lock, so
    /// a slow journal stalls no reader).
    journal_order: Arc<Mutex<()>>,
    /// Key of the session this registry belongs to; stamped on every
    /// record it creates.
    session: Arc<str>,
}

impl EnvironmentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry whose refs and records are durable through `journal`,
    /// creating records on behalf of `session`.
    pub fn with_journal(journal: EnvironmentJournal, session: &str) -> Self {
        Self {
            state: Arc::default(),
            journal: Some(journal),
            journal_order: Arc::default(),
            session: Arc::from(session),
        }
    }

    /// The session this registry creates environments for.
    pub fn session(&self) -> &str {
        &self.session
    }

    /// Whether the registry's refs and records are durable.
    pub fn is_durable(&self) -> bool {
        self.journal.is_some()
    }

    /// Mint the next `CN` ref. Refs are monotonic and never reused within a
    /// session, even when the launch they were minted for later fails or the
    /// environment is stopped. A durable registry allocates through its
    /// journal, so the ref is unique across every session of the base
    /// directory; when the journal cannot allocate, minting is refused
    /// (review F9, #2033) — never a counter that could collide with a ref
    /// another session holds. A registry without a journal counts in
    /// memory.
    pub fn mint_ref(&self) -> Result<String, RefAllocationError> {
        let allocated = match &self.journal {
            Some(journal) => {
                Some((journal.allocate_ref)().map_err(RefAllocationError::JournalUnavailable)?)
            }
            None => None,
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.next_ref = match allocated {
            Some(number) if number > state.next_ref => number,
            _ => state.next_ref + 1,
        };
        Ok(format!("C{}", state.next_ref))
    }

    /// Commit a created environment under its minted ref. A ref numbered
    /// beyond the in-memory counter (a restored record) advances it, so a
    /// journal-less mint never collides with a seeded ref.
    pub fn commit(&self, record: EnvironmentRecord) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(number) = ref_number(&record.environment_ref) {
            state.next_ref = state.next_ref.max(number);
        }
        let environment_ref = record.environment_ref.clone();
        state.entries.insert(environment_ref.clone(), record);
        drop(state);
        self.journal_ref(&environment_ref);
    }

    /// Seed the registry with records another session wrote (#2024 S4d):
    /// they arrive `Restored` with no members (the creating session's
    /// members are unreachable here). Seeding is not a transition and is
    /// never journalled: what the restore corrected it already wrote
    /// conditionally, and writing a loaded record back whole would revert
    /// whatever another session did meanwhile. The ref counter still moves
    /// past every seeded ref.
    pub fn restore(&self, records: Vec<EnvironmentRecord>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for mut record in records {
            record.origin = EnvironmentOrigin::Restored;
            record.members.clear();
            if let Some(number) = ref_number(&record.environment_ref) {
                state.next_ref = state.next_ref.max(number);
            }
            state
                .journalled
                .insert(record.environment_ref.clone(), record.status.clone());
            state.entries.insert(record.environment_ref.clone(), record);
        }
    }

    /// Report the record under `environment_ref` as changed, after the
    /// state lock is released: the snapshot and the write happen under
    /// the journal order lock, so a later transition never reaches the
    /// journal before an earlier one. A record this session created is
    /// written as it is; a restored one is written compare-and-set on the
    /// status the journal is known to hold, and when another session has
    /// moved it on meanwhile nothing is written — their state stands, and
    /// the next write of this session expects it.
    fn journal_ref(&self, environment_ref: &str) {
        let Some(journal) = &self.journal else {
            return;
        };
        let _order = self.journal_order.lock().unwrap_or_else(|e| e.into_inner());
        let (record, expected) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                state.entries.get(environment_ref).cloned(),
                state.journalled.get(environment_ref).cloned(),
            )
        };
        let Some(record) = record else {
            return;
        };
        let expected = match record.origin {
            EnvironmentOrigin::Created => None,
            EnvironmentOrigin::Restored => expected,
        };
        let outcome = (journal.recorded)(&record, expected.as_ref());
        if record.origin != EnvironmentOrigin::Restored {
            return;
        }
        let known = match outcome {
            JournalWrite::Written => record.status,
            JournalWrite::Superseded { current } => current,
            JournalWrite::Unavailable => return,
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.journalled.insert(environment_ref.to_string(), known);
    }

    /// Remove a committed environment. Used only when the launch that
    /// CREATED the environment rolls back (before or after registration):
    /// the environment never became usable, so no stopped record is listed.
    /// A stopped environment stays listed and its ref is never reused.
    pub fn remove(&self, environment_ref: &str) -> Option<EnvironmentRecord> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Prune the removed environment's inspect bookkeeping with it; refs
        // are never reused, so nothing can resurrect these keys.
        state
            .inspect_claims
            .retain(|(env_ref, _)| env_ref != environment_ref);
        state.inspect_failures.remove(environment_ref);
        let removed = state.entries.remove(environment_ref);
        drop(state);
        if removed.is_some()
            && let Some(journal) = &self.journal
        {
            let _order = self.journal_order.lock().unwrap_or_else(|e| e.into_inner());
            (journal.forgotten)(environment_ref);
        }
        removed
    }

    pub fn get(&self, environment_ref: &str) -> Option<EnvironmentRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.get(environment_ref).cloned()
    }

    pub fn entries(&self) -> Vec<EnvironmentRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.values().cloned().collect()
    }

    /// Resolve a target to its committed record regardless of status.
    pub fn resolve(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<EnvironmentRecord, EnvironmentLookupError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::resolve_locked(&state, target)
    }

    /// Resolve a target for joining: the environment must be live. Stopped
    /// environments fail as stopped; killing/cleanup-failed environments fail
    /// as stale. Never guesses.
    pub fn resolve_joinable(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<EnvironmentRecord, EnvironmentLookupError> {
        let record = self.resolve(target)?;
        match record.status {
            EnvironmentStatus::Running | EnvironmentStatus::Retained => Ok(record),
            EnvironmentStatus::Stopped => Err(EnvironmentLookupError::Stopped(
                record.environment_ref.clone(),
            )),
            EnvironmentStatus::Killing | EnvironmentStatus::CleanupFailed => Err(
                EnvironmentLookupError::Stale(record.environment_ref.clone()),
            ),
        }
    }

    fn resolve_locked(
        state: &EnvironmentRegistryState,
        target: &EnvironmentTarget,
    ) -> Result<EnvironmentRecord, EnvironmentLookupError> {
        match target {
            EnvironmentTarget::Ref(env_ref) => state
                .entries
                .get(env_ref)
                .cloned()
                .ok_or_else(|| EnvironmentLookupError::Unknown(env_ref.clone())),
            EnvironmentTarget::Name(name) => {
                // Only non-stopped records participate in name resolution, so
                // a name freed by a stopped environment can be reused without
                // a false ambiguity for the rest of the session.
                let named = |r: &&EnvironmentRecord| r.name.as_deref() == Some(name.as_str());
                let mut live = state
                    .entries
                    .values()
                    .filter(named)
                    .filter(|r| r.status != EnvironmentStatus::Stopped);
                match (live.next(), live.next()) {
                    (Some(record), None) => Ok(record.clone()),
                    (Some(_), Some(_)) => Err(EnvironmentLookupError::Ambiguous(name.clone())),
                    (None, _) => {
                        if state.entries.values().any(|r| named(&r)) {
                            Err(EnvironmentLookupError::Stopped(name.clone()))
                        } else {
                            Err(EnvironmentLookupError::Unknown(name.clone()))
                        }
                    }
                }
            }
        }
    }

    /// Record one member agent UUID joining the environment. Refused unless
    /// the environment is still running: a join racing a kill must fail here
    /// and let the launch roll back rather than register into a torn-down
    /// environment.
    pub fn add_member(
        &self,
        environment_ref: &str,
        agent_uuid: &str,
    ) -> Result<(), EnvironmentLookupError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .entries
            .get_mut(environment_ref)
            .ok_or_else(|| EnvironmentLookupError::Unknown(environment_ref.to_string()))?;
        match record.status {
            EnvironmentStatus::Running | EnvironmentStatus::Retained => {
                if !record.members.iter().any(|m| m == agent_uuid) {
                    record.members.push(agent_uuid.to_string());
                }
                Ok(())
            }
            EnvironmentStatus::Stopped => Err(EnvironmentLookupError::Stopped(
                record.environment_ref.clone(),
            )),
            EnvironmentStatus::Killing | EnvironmentStatus::CleanupFailed => Err(
                EnvironmentLookupError::Stale(record.environment_ref.clone()),
            ),
        }
    }

    /// Remove one member. When the environment becomes (or already is) empty
    /// and still running, this atomically claims its final cleanup: exactly
    /// one concurrent remover receives `Some(KillClaim)`.
    pub fn remove_member(
        &self,
        environment_ref: &str,
        agent_uuid: &str,
    ) -> Result<Option<KillClaim>, EnvironmentLookupError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .entries
            .get_mut(environment_ref)
            .ok_or_else(|| EnvironmentLookupError::Unknown(environment_ref.to_string()))?;
        record.members.retain(|m| m != agent_uuid);
        // A restored environment is not this session's to tear down: its
        // creating session may still hold members this registry cannot see,
        // so a joiner's exit leaves it and only an explicit kill ends it.
        if record.members.is_empty()
            && record.status == EnvironmentStatus::Running
            && record.origin == EnvironmentOrigin::Created
        {
            record.status = EnvironmentStatus::Killing;
            drop(state);
            self.journal_ref(environment_ref);
            Ok(Some(KillClaim {
                environment_ref: environment_ref.to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Claim the exclusive right to run this environment's kill operation.
    /// Refused while another claim is outstanding (no double-kill) and after a
    /// successful kill; allowed again after a failed kill (retry).
    pub fn begin_kill(&self, environment_ref: &str) -> Result<KillClaim, EnvironmentLookupError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .entries
            .get_mut(environment_ref)
            .ok_or_else(|| EnvironmentLookupError::Unknown(environment_ref.to_string()))?;
        match record.status {
            EnvironmentStatus::Running
            | EnvironmentStatus::CleanupFailed
            | EnvironmentStatus::Retained => {
                record.status = EnvironmentStatus::Killing;
                let claim = KillClaim {
                    environment_ref: record.environment_ref.clone(),
                };
                drop(state);
                self.journal_ref(environment_ref);
                Ok(claim)
            }
            EnvironmentStatus::Killing => Err(EnvironmentLookupError::Stale(
                record.environment_ref.clone(),
            )),
            EnvironmentStatus::Stopped => Err(EnvironmentLookupError::Stopped(
                record.environment_ref.clone(),
            )),
        }
    }

    /// Commit stopped after a successful kill. Members are gone by definition.
    pub fn complete_kill(&self, claim: KillClaim) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // A successful kill clears a previous KILL error, but never a
        // truthfully persisted inspect failure (#1369 slice 3).
        let keep_error = state.inspect_failures.contains(&claim.environment_ref);
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            record.status = EnvironmentStatus::Stopped;
            record.members.clear();
            if !keep_error {
                record.last_error = None;
            }
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }

    /// Claim the exclusive right to run this environment's retained inspect
    /// for one dead member. Exactly one death signal per member claims it;
    /// repeated EOF/reset for the same member claims nothing. Unknown
    /// environments claim nothing.
    pub fn begin_inspect(&self, environment_ref: &str, agent_uuid: &str) -> Option<InspectClaim> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.entries.contains_key(environment_ref) {
            return None;
        }
        let key = (environment_ref.to_string(), agent_uuid.to_string());
        if !state.inspect_claims.insert(key) {
            return None;
        }
        Some(InspectClaim {
            environment_ref: environment_ref.to_string(),
        })
    }

    /// Commit a successful inspect outcome onto the authoritative environment
    /// aggregate: the inspect result's metadata object is merged over the
    /// create-time metadata (create keys survive unless the inspect names
    /// them). Callers invoke this BEFORE member removal; the outcome also
    /// survives an environment that is already empty. A removed environment
    /// is a no-op, never a panic.
    pub fn record_inspect_success(&self, claim: InspectClaim, metadata: serde_json::Value) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // A later successful inspect supersedes a previously persisted
        // inspect failure: clear the sticky flag so the environment's most
        // recent inspect outcome is what get_containers reports, and drop
        // the stale inspect error unless a kill failure now owns last_error.
        let had_inspect_failure = state.inspect_failures.remove(&claim.environment_ref);
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            if had_inspect_failure && record.status != EnvironmentStatus::CleanupFailed {
                record.last_error = None;
            }
            match (record.metadata.as_object_mut(), metadata) {
                (Some(existing), serde_json::Value::Object(incoming)) => {
                    for (key, value) in incoming {
                        existing.insert(key, value);
                    }
                }
                (_, incoming) => record.metadata = incoming,
            }
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }

    /// Persist an inspect failure truthfully: the actionable error is
    /// retained on the aggregate, and the retained inspect argv survives so
    /// the inspect can be retried. A removed environment is a no-op.
    pub fn record_inspect_failure(&self, claim: InspectClaim, error: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.entries.contains_key(&claim.environment_ref) {
            state.inspect_failures.insert(claim.environment_ref.clone());
            if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
                record.last_error = Some(error.to_string());
            }
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }

    /// Withhold the claimed final-member kill (#1924): the environment stays
    /// alive as `Retained`, with `reason` recorded on its metadata under
    /// `retained`, until an explicit `kill_container`.
    pub fn retain(&self, claim: KillClaim, reason: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            debug_assert_eq!(record.status, EnvironmentStatus::Killing);
            record.status = EnvironmentStatus::Retained;
            record.members.clear();
            if let Some(object) = record.metadata.as_object_mut() {
                object.insert("retained".to_string(), serde_json::json!(reason));
            } else {
                record.metadata = serde_json::json!({ "retained": reason });
            }
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }

    /// Persist a retryable cleanup-failed state with an actionable error.
    pub fn fail_kill(&self, claim: KillClaim, error: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            record.status = EnvironmentStatus::CleanupFailed;
            record.last_error = Some(error.to_string());
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }
}

/// The number of a `CN` ref; `None` for anything else.
pub fn ref_number(environment_ref: &str) -> Option<u64> {
    let digits = environment_ref.strip_prefix('C')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
#[path = "environment_registry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "environment_registry_slice2_tests.rs"]
mod slice2_tests;

#[cfg(test)]
#[path = "environment_registry_slice3_tests.rs"]
mod slice3_tests;
