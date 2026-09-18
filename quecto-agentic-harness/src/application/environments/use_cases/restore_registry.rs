//! Restore the durable environment registry into a starting session
//! (#2024 S4d).
//!
//! The registry composition builds is seeded with every record the base
//! directory's store holds, each checked against the runtime before it is
//! believed: a record whose container is gone is marked `stopped` with the
//! reason as its last error (never silently dropped — the ref stays listed
//! and is never reused), a record the runtime cannot be asked about is kept
//! as recorded and reported unverified, and a kill that was in flight when
//! its session ended is a retryable `cleanup-failed`. The restored records
//! arrive without members and are never torn down by a joiner's exit.
//! A correction is written **conditionally** — only while the record on
//! file still has the status that was loaded — so a restore (or a `quecto
//! container ls`, which restores afresh) never reverts what a concurrent
//! session did in the meantime; seeded records are never written back.
//! Every transition the session makes afterwards is journalled through the
//! same store, and refs are allocated through it.
use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentJournal, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};

use super::super::dto::{CorrectionOutcome, EnvironmentLiveness, RestoredRegistry};
use super::super::ports::{EnvironmentProcess, EnvironmentRegistryStore};

pub struct RestoreRegistry {
    store: Arc<dyn EnvironmentRegistryStore>,
    process: Arc<dyn EnvironmentProcess>,
}

impl std::fmt::Debug for RestoreRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoreRegistry").finish_non_exhaustive()
    }
}

/// The last error a record gone at restore carries.
pub const GONE_AT_RESTORE: &str = "container not found at restore: the runtime reports it gone";

/// The last error a kill interrupted by its session's end carries.
pub const KILL_INTERRUPTED: &str =
    "kill was in flight when its session ended; retry kill_container";

impl RestoreRegistry {
    pub fn new(
        store: Arc<dyn EnvironmentRegistryStore>,
        process: Arc<dyn EnvironmentProcess>,
    ) -> Self {
        Self { store, process }
    }

    /// The journal a durable registry writes through: refs from the store,
    /// every record change to it. A store failure is reported, never a
    /// panic — the session keeps its in-memory registry.
    fn journal(&self) -> EnvironmentJournal {
        let allocate = Arc::clone(&self.store);
        let record = Arc::clone(&self.store);
        let forget = Arc::clone(&self.store);
        EnvironmentJournal {
            allocate_ref: Arc::new(move || match allocate.allocate_ref() {
                Ok(number) => Some(number),
                Err(error) => {
                    tracing::warn!(%error, "durable environment ref could not be allocated; using the in-memory counter");
                    None
                }
            }),
            recorded: Arc::new(move |entry: &EnvironmentRecord| {
                if let Err(error) = record.record(entry) {
                    tracing::warn!(environment_ref = %entry.environment_ref, %error, "environment record could not be persisted");
                }
            }),
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
        EnvironmentRegistry::with_journal(self.journal(), session)
    }

    /// Build the durable registry for `session`: seeded with the store's
    /// records, each judged against the runtime, and journalling through
    /// the store from here on. When the store cannot be read the registry
    /// starts empty and the report carries the read error; refs are
    /// still allocated through the store (which fails the same way), so
    /// nothing this session creates can collide with what it could not
    /// read — creates are refused instead (review F9, #2033).
    pub fn execute(&self, session: &str) -> (EnvironmentRegistry, RestoredRegistry) {
        let registry = EnvironmentRegistry::with_journal(self.journal(), session);
        let mut report = RestoredRegistry::default();
        let records = match self.store.load() {
            Ok(records) => records,
            Err(error) => {
                report.read_error = Some(format!(
                    "durable environment registry could not be read: {error}"
                ));
                return (registry, report);
            }
        };
        let mut restored = Vec::with_capacity(records.len());
        for record in records {
            let loaded_status = record.status.clone();
            let mut judged = record;
            let corrected = self.judge(&mut judged, &mut report);
            if !corrected {
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
        registry.restore(restored);
        (registry, report)
    }

    /// Correct one record against the runtime. Terminal records need no
    /// check; a live-looking one is believed only when its container is.
    /// `true` when the record was changed and must be written.
    fn judge(&self, record: &mut EnvironmentRecord, report: &mut RestoredRegistry) -> bool {
        let environment_ref = record.environment_ref.clone();
        match record.status {
            EnvironmentStatus::Stopped => false,
            EnvironmentStatus::Killing => {
                // The claim died with its session: nothing will settle it.
                record.status = EnvironmentStatus::CleanupFailed;
                record.last_error = Some(KILL_INTERRUPTED.to_string());
                report.restored.push(environment_ref);
                true
            }
            EnvironmentStatus::Running
            | EnvironmentStatus::Retained
            | EnvironmentStatus::CleanupFailed => match self.process.observe(record) {
                EnvironmentLiveness::Running => {
                    report.restored.push(environment_ref);
                    false
                }
                EnvironmentLiveness::Gone => {
                    record.status = EnvironmentStatus::Stopped;
                    record.last_error = Some(GONE_AT_RESTORE.to_string());
                    report.stopped.push(environment_ref);
                    true
                }
                EnvironmentLiveness::Unknown(reason) => {
                    report.unverified.push((environment_ref, reason));
                    false
                }
            },
        }
    }
}
