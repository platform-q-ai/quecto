//! Catalogue capability (epic #1193; target layout from #1845 on).
//!
//! The application layer is the authority for resolving catalogue sources into
//! one published, immutable snapshot generation. It defines the ports it
//! needs; infrastructure implements them; the use cases under `use_cases/`
//! answer the read and mutation triggers (UDS `list_models` first, #1845),
//! and every surface presents their DTOs. Only composition
//! (`composition::catalogue`) constructs a use case; interface holds handles.

pub mod dto;
pub mod ports;
pub mod use_cases;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::domain::catalogue::{
    Availability, AvailabilityStatus, CatalogueEntry, CatalogueSnapshot, RejectedEntry,
    SourceLayer, TransportKind, UnavailableReason, resolve_catalogue,
};

/// One named, ordered catalogue input. Infrastructure implements this port;
/// the application owns precedence between layers.
pub trait CatalogueSource: Send + Sync {
    /// Stable identifier used in structured per-source errors.
    fn id(&self) -> &str;
    /// The domain precedence layer this source feeds.
    fn layer(&self) -> SourceLayer;
    /// Load this source's entries. Parsing stays in infrastructure. A record
    /// the source cannot map into a domain entry is reported in
    /// [`SourceEntries::skipped`] instead of failing the whole layer, so one
    /// bad record cannot erase its valid neighbours.
    fn load(&self) -> Result<SourceEntries, String>;
}

/// What one source load yields: the entries that mapped cleanly plus the
/// records that did not (kept as diagnostics, never silently dropped).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceEntries {
    pub entries: Vec<CatalogueEntry>,
    pub skipped: Vec<SkippedRecord>,
}

impl From<Vec<CatalogueEntry>> for SourceEntries {
    fn from(entries: Vec<CatalogueEntry>) -> Self {
        Self {
            entries,
            skipped: Vec::new(),
        }
    }
}

/// One record a source could not map into a domain entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRecord {
    pub record: String,
    pub error: String,
}

/// Auth availability without exposing secret values. Judged per entry, not
/// per provider, matching the legacy per-record `configured` predicate: a key
/// declared for one model must not mark its siblings configured.
pub trait CredentialStatusPort: Send + Sync {
    fn credential_available(&self, entry: &CatalogueEntry) -> bool;
}

/// A source layer that could not be loaded; the layer is skipped so one
/// malformed input cannot erase the other layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueSourceError {
    pub source: String,
    pub error: String,
}

/// Outcome of one resolve run: the snapshot that is now current, entries the
/// domain rejected, and sources that failed to load.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCatalogue {
    pub snapshot: Arc<CatalogueSnapshot>,
    pub rejected: Vec<RejectedEntry>,
    pub source_errors: Vec<CatalogueSourceError>,
    /// Per-record diagnostics from sources whose other records loaded fine:
    /// `(source id, skipped record)` pairs surfaced to consumers.
    pub skipped: Vec<(String, SkippedRecord)>,
}

/// Atomic holder of the current published snapshot generation, plus the
/// last successfully loaded entries per source so one failing source
/// degrades to its own last-good contribution instead of freezing every
/// other layer's updates (#1581 review — a stale discovery cache must not
/// block a valid `models.json` edit from publishing).
#[derive(Debug, Clone)]
pub struct CatalogueSnapshotStore {
    current: Arc<RwLock<Arc<CatalogueSnapshot>>>,
    /// Last-good raw entries per source id (pre-availability derivation).
    last_good_layers: Arc<Mutex<HashMap<String, Vec<CatalogueEntry>>>>,
}

impl CatalogueSnapshotStore {
    pub fn new(initial: CatalogueSnapshot) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(initial))),
            last_good_layers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Remember one source's successfully loaded entries as its retention
    /// fallback for later resolves.
    fn remember_layer(&self, source_id: &str, entries: &[CatalogueEntry]) {
        self.last_good_layers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(source_id.to_string(), entries.to_vec());
    }

    /// The last successfully loaded entries for a source, if any resolve on
    /// this store has loaded it.
    fn last_good_layer(&self, source_id: &str) -> Option<Vec<CatalogueEntry>> {
        self.last_good_layers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(source_id)
            .cloned()
    }

    pub fn empty() -> Self {
        Self::new(CatalogueSnapshot::empty(0))
    }

    /// The current published generation, shared by reference: a read is a
    /// refcount bump, never a deep copy of the catalogue.
    pub fn current(&self) -> Arc<CatalogueSnapshot> {
        // A panic elsewhere must not turn every later catalogue read into a
        // process-killing panic: the snapshot behind a poisoned lock is still
        // a fully published generation (`publish` replaces it wholesale).
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Atomically replace the current snapshot. Readers see either the old or
    /// the new generation, never a partial state.
    pub fn publish(&self, snapshot: CatalogueSnapshot) {
        *self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(snapshot);
    }

    /// Resolve and publish the next generation atomically: the write lock is
    /// held across reading the current generation, building its successor,
    /// and publishing it, so two concurrent resolves can never publish two
    /// different snapshots tagged with the same generation.
    pub fn publish_next<T>(
        &self,
        resolve: impl FnOnce(u64) -> (CatalogueSnapshot, T),
    ) -> (Arc<CatalogueSnapshot>, T) {
        let mut guard = self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (snapshot, extra) = resolve(guard.generation() + 1);
        let snapshot = Arc::new(snapshot);
        *guard = snapshot.clone();
        (snapshot, extra)
    }
}

/// Resolve effective catalogue: load all sources, validate via domain rules,
/// derive availability from credential status, publish one immutable
/// generation. When every source fails, the last valid snapshot is retained.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResolveCatalogueUseCase;

impl ResolveCatalogueUseCase {
    pub fn resolve_and_publish(
        &self,
        sources: &[&dyn CatalogueSource],
        credentials: &dyn CredentialStatusPort,
        store: &CatalogueSnapshotStore,
    ) -> ResolvedCatalogue {
        let mut layers = Vec::with_capacity(sources.len());
        let mut source_errors = Vec::new();
        let mut skipped = Vec::new();
        let mut fresh_loads = 0usize;
        for source in sources {
            match source.load() {
                Ok(loaded) => {
                    skipped.extend(
                        loaded
                            .skipped
                            .into_iter()
                            .map(|record| (source.id().to_string(), record)),
                    );
                    store.remember_layer(source.id(), &loaded.entries);
                    fresh_loads += 1;
                    layers.push((source.layer(), loaded.entries));
                }
                Err(error) => {
                    source_errors.push(CatalogueSourceError {
                        source: source.id().to_string(),
                        error,
                    });
                    // Per-source retention (#1581 review): a source that
                    // fails to load degrades to its own last-good entries on
                    // this store, so the other layers' valid updates still
                    // publish (#1575, AC4b — a malformed user file keeps its
                    // last valid contribution, while an unrelated edit or a
                    // broken discovery cache never freezes the catalogue).
                    // A source that never loaded on this store contributes
                    // nothing (malformed-source isolation keeps built-ins
                    // resolving on a broken first read).
                    if let Some(entries) = store.last_good_layer(source.id()) {
                        layers.push((source.layer(), entries));
                    }
                }
            }
        }
        // When every source failed there is nothing fresh to describe: the
        // last valid generation is retained wholesale (never republished as
        // a spurious new generation) and the errors are surfaced.
        if fresh_loads == 0 && !source_errors.is_empty() {
            return ResolvedCatalogue {
                snapshot: store.current(),
                rejected: Vec::new(),
                source_errors,
                skipped,
            };
        }
        for (_, entries) in &mut layers {
            for entry in entries {
                let credential_available = credentials.credential_available(entry);
                entry.model.availability = derive_availability(
                    entry.provider.transport.clone(),
                    transport_has_adapter(&entry.provider.transport),
                    credential_available,
                );
            }
        }
        // Read-increment-resolve-publish happens under one write lock so a
        // concurrent resolve on the same store can never tag a different
        // snapshot with the same generation.
        let (snapshot, rejected) = store.publish_next(|generation| {
            let resolution = resolve_catalogue(generation, layers);
            (resolution.snapshot, resolution.rejected)
        });
        ResolvedCatalogue {
            snapshot,
            rejected,
            source_errors,
            skipped,
        }
    }
}

/// Whether a transport adapter exists for this transport kind. Every named
/// kind the domain enumerates has an adapter today; a transport a catalogue
/// file declared that no adapter implements does not (#1575, AC3), so its
/// entries stay known-but-unrunnable by construction.
fn transport_has_adapter(transport: &TransportKind) -> bool {
    !matches!(transport, TransportKind::Unsupported { .. })
}

/// Translate adapter support and credential status into derived availability.
pub fn derive_availability(
    transport: TransportKind,
    adapter_supported: bool,
    credential_available: bool,
) -> Availability {
    if adapter_supported && credential_available {
        return Availability::runnable();
    }
    let mut reasons = Vec::new();
    if !credential_available {
        reasons.push(UnavailableReason::MissingCredential);
    }
    if !adapter_supported {
        reasons.push(UnavailableReason::UnsupportedTransport { transport });
    }
    // No adapter caps the ladder at Configured; a missing credential alone
    // leaves the entry Available (everything but the credential is in place).
    let status = if adapter_supported {
        AvailabilityStatus::Available
    } else {
        AvailabilityStatus::Configured
    };
    Availability::unavailable(status, reasons)
        .expect("non-runnable status with at least one reason is always constructible")
}

/// The per-model limits a qualified `provider/model` reference declares in one
/// snapshot: `(output cap, context window)`, each `None` when not explicitly
/// declared. Every limits read goes through this one rule so a session cannot
/// silently gain or lose a clamp.
pub fn model_limits_in(
    snapshot: &CatalogueSnapshot,
    qualified: &str,
) -> (Option<u32>, Option<usize>) {
    let Ok(reference) = crate::domain::catalogue::ModelRef::parse_qualified(qualified) else {
        return (None, None);
    };
    let Some(entry) = snapshot.find(&reference) else {
        return (None, None);
    };
    // A synthesized default is not a real limit and must not clamp: only
    // explicitly declared values count.
    let capabilities = &entry.model.capabilities;
    (
        capabilities
            .max_output_tokens_explicit
            .then_some(capabilities.max_output_tokens),
        capabilities
            .context_window_explicit
            .then_some(capabilities.context_window as usize),
    )
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
