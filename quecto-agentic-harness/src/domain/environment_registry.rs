//! Session-scoped registry of script-managed environments.
//!
//! Per ADR-0021 composition builds exactly one registry per session and
//! injects it into the launch services. It is the authority for minting
//! `C1`-style environment refs — unique among everything still held, on
//! file or here (#2070) — and for recording which
//! environments this session has committed: hidden environment UUID, optional
//! name, script/runtime identity, retained script argv, member agent UUIDs,
//! status, metadata, and last error (#1369 slice 2).
//!
//! Since #2024 S4d the registry is durable: every transition is observed by
//! an optional [`EnvironmentJournal`] the application installs over its
//! store, refs are allocated through it (unique across every session of one
//! base directory), and a harness restart seeds the registry with the
//! records of earlier sessions as *restored* records — reachable for a
//! join, a listing or a kill, but never torn down by a joiner's exit. A
//! startup read that failed is retried on the next lookup (round 3 L2,
//! #2033), so a document repaired in place is seen without a restart.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

mod record_lookup;
pub use record_lookup::{EnvironmentLookupError, EnvironmentStatus, EnvironmentTarget};

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

pub use super::environment_journal::{
    EnvironmentJournal, JournalWrite, RecordedFn, RefAllocationError, merge_metadata,
};

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
    /// Refs whose [`KillClaim`] this session holds. While one is
    /// outstanding the claim is the authority on the record's status: a
    /// journal write it supersedes updates what the journal is known to
    /// hold, never the in-memory `killing` (round 2 F-A, #2033).
    kill_claims: std::collections::BTreeSet<String>,
    /// Why the durable store could not be read at startup (round 2 F-B,
    /// #2033): set, the registry holds only what this session created,
    /// every other lookup answers with this error, and the listing
    /// carries it as a diagnostic — until a lookup finds the store
    /// readable again (round 3 L2): the journal's reload then seeds what
    /// it holds and the error clears.
    read_error: Option<Arc<str>>,
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

    /// A durable registry whose store could not be read (round 2 F-B,
    /// #2033): it journals what this session creates like any other, but
    /// resolves nothing it did not create — those lookups answer with
    /// `error`, the store's own account — and reports the error through
    /// [`Self::read_error`].
    pub fn unreadable(journal: EnvironmentJournal, session: &str, error: &str) -> Self {
        let registry = Self::with_journal(journal, session);
        registry
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_error = Some(Arc::from(error));
        registry
    }

    /// Why the durable store still cannot be read, when it cannot: what
    /// the listing shows and every miss answers with. Asking retries the
    /// read (round 3 L2, #2033), so a document repaired in place is seen
    /// — and seeded — without a restart.
    pub fn read_error(&self) -> Option<String> {
        self.recover_unread();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.read_error.as_deref().map(str::to_string)
    }

    /// Retry the startup read while it is known to have failed (round 3
    /// L2, #2033). A readable store seeds the records this session did
    /// not create (what it created stands as it is) and clears the error;
    /// one still unreadable refreshes the error with the store's current
    /// account. A registry that never failed to read does nothing.
    fn recover_unread(&self) {
        let unread = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.read_error.is_some()
        };
        let Some(journal) = &self.journal else {
            return;
        };
        if !unread {
            return;
        }
        // Serialised with the journal's writes: nothing this session
        // records slips between the reload and the seeding.
        let _order = self.journal_order.lock().unwrap_or_else(|e| e.into_inner());
        match (journal.reload)() {
            Ok(records) => {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let fresh: Vec<EnvironmentRecord> = records
                    .into_iter()
                    .filter(|record| !state.entries.contains_key(&record.environment_ref))
                    .collect();
                Self::seed_locked(&mut state, fresh);
                state.read_error = None;
            }
            Err(error) => {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.read_error = Some(Arc::from(error.as_str()));
            }
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

    /// Mint the next `CN` ref. A ref is never reused while anything holds it
    /// — a record here, whatever its status, a record on file, or a mint in
    /// flight; a journal-less registry counts on in memory. A durable
    /// registry allocates through its journal, so the ref is unique across
    /// every session of the base directory; when the journal cannot
    /// allocate, minting is refused (review F9, #2033) — never a counter
    /// that could collide with a ref another session holds.
    ///
    /// The journal hands out the lowest number nothing holds (#2070: once
    /// everything is collected the next container is `C1` again), and is
    /// told the floor — one above every ref still held here — so a record
    /// the file has forgotten (another process's gc) is never reissued to
    /// this session; the number comes back reserved on file.
    pub fn mint_ref(&self) -> Result<String, RefAllocationError> {
        let Some(journal) = &self.journal else {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.next_ref += 1;
            return Ok(format!("C{}", state.next_ref));
        };
        let floor = self.highest_held().checked_add(1).ok_or_else(|| {
            RefAllocationError::JournalUnavailable("this session's ref space is exhausted".into())
        })?;
        let number =
            (journal.allocate_ref)(floor).map_err(RefAllocationError::JournalUnavailable)?;
        // A store that ignores the floor is not one this session can trust
        // with a number: refused (and given back), never counted from memory.
        if number < floor {
            (journal.release_ref)(number);
            return Err(RefAllocationError::JournalUnavailable(format!(
                "the registry minted {number} below this session's floor {floor}; it is not counting"
            )));
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.next_ref = number;
        Ok(format!("C{number}"))
    }

    /// Give a minted ref back (#2070): the create it was minted for failed
    /// before anything was committed under it.
    pub fn release_ref(&self, environment_ref: &str) {
        if let (Some(journal), Some(number)) = (&self.journal, ref_number(environment_ref)) {
            (journal.release_ref)(number);
        }
    }

    /// The highest numbered ref a record still holds here, `0` for none.
    fn highest_held(&self) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .entries
            .keys()
            .filter_map(|key| ref_number(key))
            .max()
            .unwrap_or(0)
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
        Self::seed_locked(&mut state, records);
    }

    fn seed_locked(state: &mut EnvironmentRegistryState, records: Vec<EnvironmentRecord>) {
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
        let (known, adopted) = match outcome {
            JournalWrite::Written => (record.status, None),
            JournalWrite::Superseded { current, metadata } => (current, Some(metadata)),
            JournalWrite::Unavailable => return,
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // What superseded the write stands in memory as well (round 2
        // F-A, #2033): a later non-transition write (an inspect outcome)
        // then carries the file's status, never a stale one back onto the
        // file, and the listing shows what stands. Except while this
        // session holds the record's kill claim — the claim is the
        // authority until `complete_kill`/`fail_kill`/`retain` settle it
        // (a claim's write superseded by a `stopped` elsewhere still
        // settles as this session's kill found things).
        // The file's metadata comes with it (round 3, #2033): the other
        // session's keys are adopted under this session's own, so a later
        // write of this record carries them back rather than dropping
        // them.
        if let Some(metadata) = adopted
            && !state.kill_claims.contains(environment_ref)
            && let Some(record) = state.entries.get_mut(environment_ref)
        {
            record.status = known.clone();
            record.metadata = merge_metadata(metadata, &record.metadata);
        }
        state.journalled.insert(environment_ref.to_string(), known);
    }

    /// Remove a committed environment. Used only when the launch that
    /// CREATED the environment rolls back (before or after registration):
    /// the environment never became usable, so no stopped record is listed.
    /// A stopped environment stays listed and its ref is not reused while it
    /// is.
    pub fn remove(&self, environment_ref: &str) -> Option<EnvironmentRecord> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Prune the removed environment's inspect bookkeeping with it: the
        // ref may be minted again (#2070), and a new environment under it
        // starts with no claims or failures.
        state
            .inspect_claims
            .retain(|(env_ref, _)| env_ref != environment_ref);
        state.inspect_failures.remove(environment_ref);
        state.kill_claims.remove(environment_ref);
        let removed = state.entries.remove(environment_ref);
        drop(state);
        if let Some(record) = &removed
            && let Some(journal) = &self.journal
        {
            let _order = self.journal_order.lock().unwrap_or_else(|e| e.into_inner());
            (journal.forgotten)(record);
        }
        removed
    }

    pub fn get(&self, environment_ref: &str) -> Option<EnvironmentRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.get(environment_ref).cloned()
    }

    /// Every record, seeded afresh first when the store was unreadable
    /// at startup and can be read now (round 3 L2, #2033).
    pub fn entries(&self) -> Vec<EnvironmentRecord> {
        self.recover_unread();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.values().cloned().collect()
    }

    /// Resolve a target to its committed record regardless of status.
    pub fn resolve(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<EnvironmentRecord, EnvironmentLookupError> {
        self.recover_unread();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match (Self::resolve_locked(&state, target), &state.read_error) {
            // Nothing is known of what the store holds: a miss is not an
            // unknown target, it is the store's read error (round 2 F-B).
            (Err(EnvironmentLookupError::Unknown(_)), Some(error)) => {
                Err(EnvironmentLookupError::Unreadable(error.to_string()))
            }
            (resolved, _) => resolved,
        }
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
            state.kill_claims.insert(environment_ref.to_string());
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
    /// successful kill; allowed again after a failed kill (retry). A
    /// restored record found `killing` holds another session's claim,
    /// which this registry cannot see settle: an explicit kill here is the
    /// operator's retry and claims it (review F6, #2033).
    pub fn begin_kill(&self, environment_ref: &str) -> Result<KillClaim, EnvironmentLookupError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .entries
            .get_mut(environment_ref)
            .ok_or_else(|| EnvironmentLookupError::Unknown(environment_ref.to_string()))?;
        let claimable = match record.status {
            EnvironmentStatus::Running
            | EnvironmentStatus::CleanupFailed
            | EnvironmentStatus::Retained => true,
            EnvironmentStatus::Killing => record.origin == EnvironmentOrigin::Restored,
            EnvironmentStatus::Stopped => false,
        };
        if claimable {
            record.status = EnvironmentStatus::Killing;
            let claim = KillClaim {
                environment_ref: record.environment_ref.clone(),
            };
            state.kill_claims.insert(environment_ref.to_string());
            drop(state);
            self.journal_ref(environment_ref);
            return Ok(claim);
        }
        match record.status {
            EnvironmentStatus::Stopped => Err(EnvironmentLookupError::Stopped(
                record.environment_ref.clone(),
            )),
            // This session's own claim is outstanding: no double-kill.
            EnvironmentStatus::Killing
            | EnvironmentStatus::Running
            | EnvironmentStatus::CleanupFailed
            | EnvironmentStatus::Retained => Err(EnvironmentLookupError::Stale(
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
        state.kill_claims.remove(&claim.environment_ref);
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

    /// Withhold the claimed final-member kill (#1924): the environment stays
    /// alive as `Retained`, with `reason` recorded on its metadata under
    /// `retained`, until an explicit `kill_container`.
    pub fn retain(&self, claim: KillClaim, reason: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.kill_claims.remove(&claim.environment_ref);
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            debug_assert_eq!(record.status, EnvironmentStatus::Killing);
            record.retain_with(reason);
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }

    /// Persist a retryable cleanup-failed state with an actionable error.
    pub fn fail_kill(&self, claim: KillClaim, error: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.kill_claims.remove(&claim.environment_ref);
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

#[path = "environment_registry_inspect.rs"]
mod inspect;

#[cfg(test)]
#[path = "environment_registry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "environment_registry_slice2_tests.rs"]
mod slice2_tests;

#[cfg(test)]
#[path = "environment_registry_slice3_tests.rs"]
mod slice3_tests;
