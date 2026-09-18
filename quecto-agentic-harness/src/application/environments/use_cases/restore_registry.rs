//! Restore the durable environment registry into a starting session
//! (#2024 S4d).
//!
//! The registry composition builds is seeded with every record the base
//! directory's store holds, each checked against the runtime before it is
//! believed: a record whose container is gone is marked `stopped` with the
//! reason as its last error (never silently dropped — the ref stays listed
//! and is never reused), a record the runtime cannot be asked about is kept
//! as recorded and reported unverified, and a kill that was in flight when
//! this session started is reported as such — never relabelled, since its
//! session may still be live and settling it (review F6, #2033); an
//! explicit kill from here retries it. The restored records
//! arrive without members and are never torn down by a joiner's exit.
//! A correction is written **conditionally** — only while the record on
//! file still has the status that was loaded — so a restore (or a `quecto
//! container ls`, which restores afresh) never reverts what a concurrent
//! session did in the meantime; seeded records are never written back.
//! Every transition the session makes afterwards is journalled through the
//! same store, and refs are allocated through it.
use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentJournal, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, JournalWrite,
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

/// The reason a `killing` record is reported unverified at restore.
pub const KILL_IN_FLIGHT: &str =
    "kill in flight (its session may be live and settling it); kill_container retries it";

impl RestoreRegistry {
    pub fn new(
        store: Arc<dyn EnvironmentRegistryStore>,
        process: Arc<dyn EnvironmentProcess>,
    ) -> Self {
        Self { store, process }
    }

    /// The journal a durable registry writes through: refs from the store
    /// (a store that cannot allocate refuses the mint — review F9, #2033),
    /// every record change to it. A failed write is reported, never a
    /// panic — the session keeps its in-memory registry. A write made with
    /// an expected status is the store's conditional `correct` (review F5):
    /// applied only while the record on file still has that status.
    fn journal(&self) -> EnvironmentJournal {
        let allocate = Arc::clone(&self.store);
        let record = Arc::clone(&self.store);
        let forget = Arc::clone(&self.store);
        EnvironmentJournal {
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
                                    }
                                }
                                // Forgotten by its creator (a rolled-back
                                // create): nothing of it to keep.
                                CorrectionOutcome::Forgotten => JournalWrite::Superseded {
                                    current: EnvironmentStatus::Stopped,
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
        let mut report = RestoredRegistry::default();
        let records = match self.store.load() {
            Ok(records) => records,
            Err(error) => {
                let read_error = format!("durable environment registry could not be read: {error}");
                // The registry carries the error itself (round 2 F-B,
                // #2033): the model's listing shows it and a lookup of
                // anything this session did not create answers with it.
                let registry =
                    EnvironmentRegistry::unreadable(self.journal(), session, &read_error);
                report.read_error = Some(read_error);
                return (registry, report);
            }
        };
        let registry = EnvironmentRegistry::with_journal(self.journal(), session);
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
                // Whether its session is still settling the kill cannot
                // be told from here: say so, change nothing.
                report
                    .unverified
                    .push((environment_ref, KILL_IN_FLIGHT.to_string()));
                false
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
