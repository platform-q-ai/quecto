//! Restore the durable environment registry into a starting session
//! (#2024 S4d).
//!
//! The registry composition builds is seeded with every record the base
//! directory's store holds, each checked against the runtime before it is
//! believed: a `running` or `cleanup-failed` record whose container is
//! gone is marked `stopped` with the reason as its last error (never
//! silently dropped — the ref stays listed and is never reused), a record
//! the runtime cannot be asked about is kept as recorded and reported
//! unverified, and a kill that was in flight when this session started is
//! reported as such — never relabelled, since its session may still be
//! live and settling it (review F6, #2033); an explicit kill from here
//! retries it. A `retained` record (#1924) is never relabelled either
//! (round 3 H1, #2033): under the shipped adapter its container has
//! exited by design, only an explicit kill ends it, and calling it
//! `stopped` would hand its state dir to the collector — it is reported.
//! A `running` record whose container is gone but whose checkout hosts an
//! **unfinished** swarm run (the master exited before its coordinator
//! did, so nobody finalized the member; the in-container harness then ran
//! its parent-loss shutdown and the box exited) is relabelled `retained`
//! with the reason the finalizer would have recorded, never `stopped`
//! (round 4 M1, #2033); a store that cannot be read retains as the
//! finalizer would too. And a record an older build relabelled `stopped`
//! while it was retained — recognisable by its `metadata.retained` under a
//! `stopped` status with the restore's own last error — is restored to
//! `retained`, so an explicit kill can end it (round 4 L3).
//! The restored records arrive without members and are never torn down
//! by a joiner's exit. A correction is written **conditionally** — only
//! while the record on file still has the status that was loaded — so a
//! restore (or a `quecto container ls`, which restores afresh) never
//! reverts what a concurrent session did in the meantime; seeded records
//! are never written back. An *observing* restore (`gc --dry-run`) seeds
//! the same corrections in memory and writes none. Every transition the
//! session makes afterwards is journalled through the same store, and
//! refs are allocated through it; a startup read that failed is retried
//! on the session's next lookup (round 3 L2), so an in-place repair of
//! the document is seen without a restart.
use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentJournal, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, JournalWrite,
};

use crate::domain::environment_retention::{HostedSwarmRun, SwarmRunObservation};

use super::super::dto::{CorrectionOutcome, EnvironmentLiveness, RestoreMode, RestoredRegistry};
use super::super::ports::{EnvironmentProcess, EnvironmentRegistryStore, HostedSwarmRunInspection};

#[derive(Clone)]
pub struct RestoreRegistry {
    store: Arc<dyn EnvironmentRegistryStore>,
    process: Arc<dyn EnvironmentProcess>,
    hosted: Arc<dyn HostedSwarmRunInspection>,
}

impl std::fmt::Debug for RestoreRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoreRegistry").finish_non_exhaustive()
    }
}

/// The last error a record gone at restore carries.
pub const GONE_AT_RESTORE: &str = "container not found at restore: the runtime reports it gone";

/// The reason a `killing` record is reported unverified at restore.
pub const KILL_IN_FLIGHT: &str =
    "kill in flight (its session may be live and settling it); kill_container retries it";

/// The reason a `retained` record whose container has exited is reported
/// unverified at restore (round 3 H1, #2033): that is its normal state.
pub const RETAINED_EXITED: &str = "retained: container exited; only container kill ends it";

/// The reason a record an older build relabelled `stopped` while it was
/// retained is restored to `retained` (round 4 L3, #2033).
pub const RELABELLED_BY_OLDER_BUILD: &str = "was retained; relabelled stopped by an older build's restore — restored to retained; kill explicitly to collect";

const KEPT: &str = "environment retained for inspection, kill_container to remove";

/// The `metadata.retained` reason for a gone container whose checkout
/// hosts an unfinished run (round 4 M1, #2033): what the finalizer would
/// have recorded had the master seen the coordinator go.
pub fn unfinished_run_reason(run: &HostedSwarmRun) -> String {
    format!(
        "run {} unfinished ({}); container exited; {KEPT}",
        run.id,
        run.describe()
    )
}

fn unreadable_store_reason(error: &str) -> String {
    format!("coordination store could not be read ({error}); container exited; {KEPT}")
}

impl RestoreRegistry {
    pub fn new(
        store: Arc<dyn EnvironmentRegistryStore>,
        process: Arc<dyn EnvironmentProcess>,
        hosted: Arc<dyn HostedSwarmRunInspection>,
    ) -> Self {
        Self {
            store,
            process,
            hosted,
        }
    }

    /// The journal a durable registry writes through: refs from the store
    /// (a store that cannot allocate refuses the mint — review F9, #2033),
    /// every record change to it. A failed write is reported, never a
    /// panic — the session keeps its in-memory registry. A write made with
    /// an expected status is the store's conditional `correct` (review F5):
    /// applied only while the record on file still has that status.
    fn journal(&self, mode: RestoreMode) -> EnvironmentJournal {
        let allocate = Arc::clone(&self.store);
        let record = Arc::clone(&self.store);
        let forget = Arc::clone(&self.store);
        let reload = self.clone();
        EnvironmentJournal {
            reload: Arc::new(move || {
                let records = reload
                    .store
                    .load()
                    .map_err(|error| format!("{READ_FAILED}: {error}"))?;
                // The startup read has succeeded late (round 3 L2): the
                // records are judged and corrected as they would have
                // been then; the account goes to the log, there being
                // no report to carry it.
                let mut report = RestoredRegistry::default();
                let seeded = reload.seed(records, mode, &mut report);
                for line in &report.diagnostics {
                    tracing::info!(%line, "durable environment registry read late");
                }
                tracing::info!(
                    restored = report.restored.len(),
                    stopped = report.stopped.len(),
                    retained = report.retained.len(),
                    unverified = report.unverified.len(),
                    "durable environment registry readable again; seeded"
                );
                Ok(seeded)
            }),
            allocate_ref: Arc::new(move || {
                allocate.allocate_ref().map_err(|error| {
                    tracing::warn!(%error, "durable environment ref could not be allocated; the create is refused");
                    error
                })
            }),
            recorded: Arc::new(
                move |entry: &EnvironmentRecord, expected: Option<&EnvironmentStatus>| {
                    let outcome = match expected {
                        None => record.record(entry).map(|()| JournalWrite::Written),
                        Some(expected) => {
                            record.correct(entry, expected).map(|outcome| match outcome {
                                CorrectionOutcome::Applied => JournalWrite::Written,
                                CorrectionOutcome::Superseded(current) => {
                                    tracing::info!(environment_ref = %entry.environment_ref, expected = ?expected, current = current.status_label(), "another session moved the environment on; its state stands");
                                    JournalWrite::Superseded {
                                        current: current.status,
                                        metadata: current.metadata,
                                    }
                                }
                                // Forgotten by its creator (a rolled-back
                                // create): nothing of it to keep.
                                CorrectionOutcome::Forgotten => JournalWrite::Superseded {
                                    current: EnvironmentStatus::Stopped,
                                    metadata: entry.metadata.clone(),
                                },
                            })
                        }
                    };
                    outcome.unwrap_or_else(|error| {
                        tracing::warn!(environment_ref = %entry.environment_ref, %error, "environment record could not be persisted");
                        JournalWrite::Unavailable
                    })
                },
            ),
            forgotten: Arc::new(move |environment_ref: &str| {
                if let Err(error) = forget.forget(environment_ref) {
                    tracing::warn!(environment_ref, %error, "environment record could not be removed from the durable registry");
                }
            }),
        }
    }

    /// A durable registry for `session` that is not seeded: it allocates
    /// its refs through the store and journals every record it creates, but
    /// inherits nothing (a spawned child's registry — the fleet is its
    /// parent's to show).
    pub fn unseeded(&self, session: &str) -> EnvironmentRegistry {
        EnvironmentRegistry::with_journal(self.journal(RestoreMode::Correct), session)
    }

    /// Build the durable registry for `session`: seeded with the store's
    /// records, each judged against the runtime, and journalling through
    /// the store from here on. When the store cannot be read the registry
    /// starts empty and the report carries the read error; refs are
    /// still allocated through the store (which fails the same way), so
    /// nothing this session creates can collide with what it could not
    /// read — creates are refused instead (review F9, #2033).
    pub fn execute(&self, session: &str) -> (EnvironmentRegistry, RestoredRegistry) {
        self.execute_as(session, RestoreMode::Correct)
    }

    /// [`Self::execute`] without an effect on the store (round 3 H1,
    /// #2033): every correction is seeded in memory and reported as what
    /// a correcting restore would write. For a registry that only reads
    /// — a `container gc --dry-run` — so the preview matches the real run
    /// and the document is left byte for byte as it was.
    pub fn observe(&self, session: &str) -> (EnvironmentRegistry, RestoredRegistry) {
        self.execute_as(session, RestoreMode::Observe)
    }

    fn execute_as(
        &self,
        session: &str,
        mode: RestoreMode,
    ) -> (EnvironmentRegistry, RestoredRegistry) {
        let mut report = RestoredRegistry::default();
        let records = match self.store.load() {
            Ok(records) => records,
            Err(error) => {
                let read_error = format!("{READ_FAILED}: {error}");
                // The registry carries the error itself (round 2 F-B,
                // #2033): the model's listing shows it and a lookup of
                // anything this session did not create answers with it.
                let registry =
                    EnvironmentRegistry::unreadable(self.journal(mode), session, &read_error);
                report.read_error = Some(read_error);
                return (registry, report);
            }
        };
        let registry = EnvironmentRegistry::with_journal(self.journal(mode), session);
        registry.restore(self.seed(records, mode, &mut report));
        (registry, report)
    }

    /// The store's records judged against the runtime, each correction
    /// handled as `mode` says: the records to seed the registry with.
    fn seed(
        &self,
        records: Vec<EnvironmentRecord>,
        mode: RestoreMode,
        report: &mut RestoredRegistry,
    ) -> Vec<EnvironmentRecord> {
        let mut restored = Vec::with_capacity(records.len());
        for record in records {
            let loaded_status = record.status.clone();
            let mut judged = record;
            let corrected = self.judge(&mut judged, report);
            if !corrected {
                restored.push(judged);
                continue;
            }
            if mode == RestoreMode::Observe {
                let why = judged
                    .last_error
                    .as_deref()
                    .or_else(|| judged.metadata.get("retained").and_then(|v| v.as_str()))
                    .unwrap_or("corrected");
                report.diagnostics.push(format!(
                    "{} would be recorded {} ({why}); not written: this restore only observes",
                    judged.environment_ref,
                    judged.status_label(),
                ));
                restored.push(judged);
                continue;
            }
            // Written only while nobody else moved the record on; when
            // somebody did, what they wrote is the truth to seed with.
            match self.store.correct(&judged, &loaded_status) {
                Ok(CorrectionOutcome::Applied) => restored.push(judged),
                Ok(CorrectionOutcome::Superseded(current)) => {
                    report.diagnostics.push(format!(
                        "{} changed while it was being checked ({} → {}); the other session's state stands",
                        judged.environment_ref,
                        judged.status_label(),
                        current.status_label()
                    ));
                    restored.push(*current);
                }
                Ok(CorrectionOutcome::Forgotten) => {}
                Err(error) => {
                    report.diagnostics.push(format!(
                        "{} could not be corrected in the durable registry: {error}",
                        judged.environment_ref
                    ));
                    restored.push(judged);
                }
            }
        }
        restored
    }

    /// Correct one record against the runtime. Terminal records need no
    /// check; a live-looking one is believed only when its container is.
    /// `true` when the record was changed and must be written.
    fn judge(&self, record: &mut EnvironmentRecord, report: &mut RestoredRegistry) -> bool {
        let environment_ref = record.environment_ref.clone();
        match record.status {
            // An older build relabelled a retained record at restore
            // (round 4 L3): the record's own `retained` reason under a
            // `stopped` status with this restore's last error is that
            // build's signature and nothing else's (an explicit kill
            // clears the last error). Restored, so a kill can end it.
            EnvironmentStatus::Stopped if relabelled_by_older_build(record) => {
                record.status = EnvironmentStatus::Retained;
                record.last_error = None;
                report
                    .retained
                    .push((environment_ref, RELABELLED_BY_OLDER_BUILD.to_string()));
                true
            }
            EnvironmentStatus::Stopped => false,
            EnvironmentStatus::Killing => {
                // Whether its session is still settling the kill cannot
                // be told from here: say so, change nothing.
                report
                    .unverified
                    .push((environment_ref, KILL_IN_FLIGHT.to_string()));
                false
            }
            EnvironmentStatus::Running
            | EnvironmentStatus::Retained
            | EnvironmentStatus::CleanupFailed => {
                match (self.process.observe(record), &record.status) {
                    (EnvironmentLiveness::Running, _) => {
                        report.restored.push(environment_ref);
                        false
                    }
                    // Retained (#1924): an exited container is how it is kept
                    // — the board and checkout live in its state dir, and only
                    // an explicit kill ends it (round 3 H1, #2033). Said, not
                    // relabelled, so no collector ever sees it as stopped.
                    (EnvironmentLiveness::Gone, EnvironmentStatus::Retained) => {
                        report
                            .unverified
                            .push((environment_ref, RETAINED_EXITED.to_string()));
                        false
                    }
                    // The master exited before its coordinator did (round 4
                    // M1): nobody finalized the member, the in-container
                    // harness ran its parent-loss shutdown and the box
                    // exited with the run unfinished. What the finalizer
                    // would have decided is decided here.
                    (EnvironmentLiveness::Gone, EnvironmentStatus::Running) => {
                        match self.retention_of_gone(record) {
                            Some(reason) => {
                                record.retain_with(&reason);
                                record.last_error = None;
                                report.retained.push((environment_ref, reason));
                            }
                            None => {
                                record.status = EnvironmentStatus::Stopped;
                                record.last_error = Some(GONE_AT_RESTORE.to_string());
                                report.stopped.push(environment_ref);
                            }
                        }
                        true
                    }
                    (EnvironmentLiveness::Gone, _) => {
                        record.status = EnvironmentStatus::Stopped;
                        // A failed kill's own error is kept behind the restore
                        // note: it is the operator's diagnosis, and its
                        // presence keeps the "relabelled by an older build"
                        // signature (`GONE_AT_RESTORE` alone) exclusive.
                        record.last_error = Some(match record.last_error.take() {
                            Some(earlier) if !earlier.is_empty() => {
                                format!("{GONE_AT_RESTORE}; earlier: {earlier}")
                            }
                            _ => GONE_AT_RESTORE.to_string(),
                        });
                        report.stopped.push(environment_ref);
                        true
                    }
                    (EnvironmentLiveness::Unknown(reason), _) => {
                        report.unverified.push((environment_ref, reason));
                        false
                    }
                }
            }
        }
    }

    /// Why a `running` record whose container is gone is retained rather
    /// than stopped, when it is: its checkout hosts a created run that has
    /// not ended, or a store that cannot be read (retained, as the
    /// finalizer retains it: a destroyed box cannot be recovered). An
    /// ended run, the bootstrap placeholder or no store: `None`.
    fn retention_of_gone(&self, record: &EnvironmentRecord) -> Option<String> {
        match self.hosted.inspect_hosted_run(record) {
            SwarmRunObservation::Run(run) if run.created() && !run.ended() => {
                Some(unfinished_run_reason(&run))
            }
            SwarmRunObservation::Run(_) | SwarmRunObservation::NoStore => None,
            SwarmRunObservation::Unreadable(error) => Some(unreadable_store_reason(&error)),
        }
    }
}

/// An older build's restore relabelled this record `stopped` while it was
/// retained (round 4 L3, #2033).
fn relabelled_by_older_build(record: &EnvironmentRecord) -> bool {
    record.status == EnvironmentStatus::Stopped
        && record
            .metadata
            .get("retained")
            .is_some_and(|v| v.is_string())
        && record.last_error.as_deref() == Some(GONE_AT_RESTORE)
}

/// How a store that could not be read is reported.
const READ_FAILED: &str = "durable environment registry could not be read";
