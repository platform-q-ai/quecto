//! Discardable discovery index. Every query validates against authority; exact
//! reads never depend on the index. Atomic replacement failures are diagnostics.
//!
//! Freshness is a file stamp (see [`stamp`]), never a content digest: a cold
//! process seeds its projection from the persisted index and reads only the
//! records whose stamp changed or which are new, so a listing over thousands
//! of transcripts costs one `stat` per file. The on-disk shape lives in
//! [`session_home_catalogue_index`].
use super::{
    session_layout::FlatSessionLayout,
    session_store::{
        FileSessionStore,
        session_store_home::{atomic_write, error},
        session_store_list::SummaryCache,
    },
};
use crate::{
    application::sessions::ports::session_home::{
        Answer, HomeCatalogueSnapshot, SessionHomeCatalogue, SessionMetadataSnapshot,
    },
    domain::{
        error::DomainError,
        session_home::{SessionHome, SessionHomeScope},
        session_identity::SessionIdentity,
    },
};
use std::{collections::BTreeMap, path::PathBuf};

#[path = "session_home_catalogue_joined.rs"]
mod session_home_catalogue_joined;
use session_home_catalogue_joined::JoinedWalk;
#[path = "session_home_catalogue_index.rs"]
mod session_home_catalogue_index;
use session_home_catalogue_index::{Catalogue, HomeEntry, IndexEntry, IndexHome, SummaryEntry};
#[path = "session_home_catalogue_metadata.rs"]
mod session_home_catalogue_metadata;
#[path = "session_home_catalogue_rejections.rs"]
pub(super) mod session_home_catalogue_rejections;
use session_home_catalogue_rejections::{Rejections, legacy_key};
#[path = "session_home_catalogue_seed.rs"]
mod session_home_catalogue_seed;
pub(super) use session_home_catalogue_seed::{WalkEntries, persisted_walk};

#[derive(Clone)]
pub struct FileSessionHomeCatalogue {
    layout: FlatSessionLayout,
    store: std::sync::Arc<FileSessionStore>,
    published: std::sync::Arc<std::sync::Mutex<Published>>,
    projection: std::sync::Arc<std::sync::Mutex<BTreeMap<PathBuf, Projection>>>,
    /// Content verdicts by stamp, in memory only; never locked with `projection`.
    rejections: std::sync::Arc<std::sync::Mutex<Rejections>>,
    transcript_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    record_stamps: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// The generation of the scan in progress or last run.
    scan: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
/// One validated record, keyed in the projection by the identity's own
/// layout path: a projection can never yield another file's identity.
#[derive(Clone)]
struct Projection {
    identity: SessionIdentity,
    entry: IndexEntry,
    /// The scan that last saw this record (#2042): the caches keep exactly
    /// the entries the latest scan touched, with no set of paths built.
    seen: u64,
}
type Records = BTreeMap<String, IndexEntry>;
/// Our last publication: its bytes, and the index they encode.
type Published = Option<(Vec<u8>, Catalogue)>;

/// Include ctime and inode, not merely user-restorable mtime/length. Only regular
/// files qualify; changed/replaced authorities must be validated again.
pub(super) fn stamp(path: &std::path::Path) -> Result<Vec<u64>, DomainError> {
    stamp_of(std::fs::symlink_metadata(path).map_err(error)?)
}
const STAMP_LEN: usize = 8;
/// A sidecar's stamp; `None` when no sidecar exists at all.
fn sidecar_stamp(path: &std::path::Path) -> Result<Option<Vec<u64>>, DomainError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(error(e)),
        Ok(metadata) => stamp_of(metadata).map(Some),
    }
}
fn stamp_of(metadata: std::fs::Metadata) -> Result<Vec<u64>, DomainError> {
    use std::os::unix::fs::MetadataExt;
    if metadata.file_type().is_file() {
        Ok(vec![
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mode() as u64,
            metadata.mtime() as u64,
            metadata.mtime_nsec() as u64,
            metadata.ctime() as u64,
            metadata.ctime_nsec() as u64,
        ])
    } else {
        Err(error("session authority is not a regular file"))
    }
}

impl FileSessionHomeCatalogue {
    /// The catalogue over `store`'s own layout: home authority is written
    /// under the store's claims, so the two never diverge.
    pub fn with_store(store: std::sync::Arc<FileSessionStore>) -> Self {
        Self {
            layout: store.layout().clone(),
            store,
            published: Default::default(),
            projection: Default::default(),
            rejections: Default::default(),
            transcript_reads: Default::default(),
            record_stamps: Default::default(),
            scan: Default::default(),
        }
    }
    /// What the caches hold, projected plus rejected (test-support).
    #[cfg(feature = "test-support")]
    pub fn remembered_records(&self) -> usize {
        let projected = self.projection.lock().map(|p| p.len()).unwrap_or(0);
        let rejected = self.rejections.lock().map(|r| r.len()).unwrap_or(0);
        projected + rejected
    }
    #[cfg(feature = "test-support")]
    pub fn transcript_reads(&self) -> usize {
        self.transcript_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    fn scan(
        &self,
        walk: &mut JoinedWalk,
    ) -> Result<(Catalogue, HomeCatalogueSnapshot), DomainError> {
        // The walk's cache, held for the scan as the walk itself holds it.
        let mut cache = SummaryCache::seeded(self.store.summary_cache(), &self.layout)?;
        let mut result = HomeCatalogueSnapshot {
            entries: Vec::new(),
            diagnostics: Vec::new(),
            rebuilt: false,
        };
        let mut records = BTreeMap::new();
        let generation = self.scan.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let entries = match std::fs::read_dir(self.layout.sessions_dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Catalogue::new(records), result));
            }
            Err(e) => return Err(error(e)),
        };
        for entry in entries {
            match entry {
                Ok(entry) if FlatSessionLayout::is_session_record(&entry.path()) => {
                    self.scan_record(&entry.path(), &mut records, &mut result, &mut cache, walk);
                }
                Ok(_) => (),
                Err(e) => result
                    .diagnostics
                    .push(format!("session directory entry unavailable: {e}")),
            }
        }
        drop(cache);
        self.publish_stable_rows(&mut records, &mut result, generation)?;
        result.entries.sort_by(|a, b| a.0.cmp(&b.0));
        walk.sort();
        Ok((Catalogue::new(records), result))
    }
    fn publish_stable_rows(
        &self,
        records: &mut Records,
        result: &mut HomeCatalogueSnapshot,
        generation: u64,
    ) -> Result<(), DomainError> {
        // Every row was stamped by the pass that read or remembered it; a
        // record rewritten since is one autosave stale, never partial, and
        // the next query re-stamps it — so publication takes no second
        // stamp (#2042). The caches keep exactly the paths this scan saw:
        // a deleted record leaves them here, with no `exists` sweep.
        let mut projection = self.projection.lock().map_err(error)?;
        for (identity, home) in &result.entries {
            debug_assert!(
                indexed_home_matches(records, identity, home),
                "published home observation must match its index entry"
            );
        }
        projection.retain(|_, cached| cached.seen == generation);
        drop(projection);
        self.rejections
            .lock()
            .map_err(error)?
            .retain_seen(generation);
        Ok(())
    }
    fn scan_record(
        &self,
        path: &std::path::Path,
        records: &mut Records,
        result: &mut HomeCatalogueSnapshot,
        cache: &mut SummaryCache,
        walk: &mut JoinedWalk,
    ) {
        let verdicts = self.verdicts(path, cache, walk);
        let summary = if verdicts.listed {
            walk.summaries.last()
        } else {
            None
        };
        match verdicts.identity {
            Ok(identity) => {
                let home = self.observe_home(path, &identity, records, summary);
                debug_assert!(
                    indexed_home_matches(records, &identity, &home),
                    "published home observation must match its index entry"
                );
                if let SessionHomeScope::Unavailable(reason) = &home {
                    // Named like a record (R1-H4): N broken sidecars are N
                    // distinguishable lines.
                    let sidecar = self.layout.home_file(&identity);
                    let file = sidecar.file_name().unwrap_or_default().to_string_lossy();
                    result
                        .diagnostics
                        .push(format!("{file}: home needs repair: {reason}"));
                }
                result.entries.push((identity, home));
            }
            // Name the record file (basename only) so a corrupt legacy
            // transcript can be found and repaired from the diagnostic alone.
            Err(e) => {
                let file = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                result
                    .diagnostics
                    .push(format!("{file}: session record unavailable: {e}"));
            }
        }
    }
    /// The home beside `path`'s record: the projected observation when the
    /// sidecar still carries the projected stamp, else one exact read, cached
    /// only when the sidecar was stable around it (a concurrent atomic home
    /// replacement can never mix two versions). Writes the record's entry.
    fn observe_home(
        &self,
        path: &std::path::Path,
        identity: &SessionIdentity,
        records: &mut Records,
        summary: Option<&crate::domain::session::SessionSummary>,
    ) -> SessionHomeScope {
        let sidecar = self.layout.home_file(identity);
        let before = sidecar_stamp(&sidecar);
        // The fresh observation is the admission read itself (`read`): the
        // listing can never see a home admission would not.
        let exact = || self.read_home_exactly(identity);
        let Ok(mut projection) = self.projection.lock() else {
            return exact();
        };
        let Some(cached) = projection.get_mut(path) else {
            // Unreachable by construction (just projected): observe exactly.
            return exact();
        };
        cached.seen = self.scan.load(std::sync::atomic::Ordering::Relaxed);
        let reusable = cached
            .entry
            .home
            .as_ref()
            .filter(|entry| before.as_ref().is_ok_and(|now| *now == entry.stamp));
        let home = match reusable {
            Some(entry) => entry.scope.to_scope(),
            None => {
                let home = exact();
                let after = sidecar_stamp(&sidecar);
                let stable = matches!((&before, &after), (Ok(b), Ok(a)) if b == a);
                cached.entry.home = match (stable, after, IndexHome::from_scope(&home)) {
                    (true, Ok(stamp), Some(scope)) => Some(HomeEntry { stamp, scope }),
                    _ => None,
                };
                home
            }
        };
        // The pass validated the walk's summary at this very stamp, or the
        // entry keeps the one it was seeded with (same stamp, same record).
        if let Some(summary) = summary {
            cached.entry.summary = Some(SummaryEntry {
                title: summary.title.clone(),
                message_count: summary.message_count,
            });
        }
        records.insert(identity.runtime_key().to_string(), cached.entry.clone());
        home
    }
    /// The exact authority read; `read_home` never fails on an unreadable
    /// sidecar (that is an `Unavailable` observation), so an error here is
    /// the same observation.
    fn read_home_exactly(&self, identity: &SessionIdentity) -> SessionHomeScope {
        self.store
            .read_home(identity)
            .unwrap_or_else(|e| SessionHomeScope::Unavailable(e.to_string()))
    }
    /// Trust the persisted index only through its stamps: every seeded entry
    /// is keyed by its own identity's layout path and reused only while the
    /// file still carries the recorded stamp.
    fn seed_projection(&self, index: &Catalogue) -> Result<(), DomainError> {
        let mut projection = self.projection.lock().map_err(error)?;
        projection.clear();
        for (key, entry) in &index.records {
            let identity = persisted_identity(key.as_str());
            if identity.persisted_key().is_some() && entry.stamp.len() == STAMP_LEN {
                projection.insert(
                    self.layout.session_file(&identity),
                    Projection {
                        identity,
                        entry: entry.clone(),
                        seen: 0,
                    },
                );
            }
        }
        Ok(())
    }
}
/// The one raw-key conversion of the catalogue and its child modules: a key
/// read from a record or the index, trusted only once its layout path agrees.
fn persisted_identity(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}
/// A row without an entry (an exact fallback observation) is no mismatch.
fn indexed_home_matches(
    records: &Records,
    identity: &SessionIdentity,
    home: &SessionHomeScope,
) -> bool {
    records
        .get(identity.runtime_key())
        .is_none_or(|entry| match &entry.home {
            Some(cached) => cached.scope.to_scope() == *home,
            None => true,
        })
}
impl SessionHomeCatalogue for FileSessionHomeCatalogue {
    fn metadata(&self) -> Answer<'_, SessionMetadataSnapshot> {
        Box::pin(session_home_catalogue_metadata::query(self.clone()))
    }
    fn list_async(&self) -> Answer<'_, HomeCatalogueSnapshot> {
        let catalogue = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || catalogue.list())
                .await
                .map_err(error)?
        })
    }
    fn read(&self, identity: &SessionIdentity) -> Result<SessionHomeScope, DomainError> {
        self.store.read_home(identity)
    }
    fn record_new(
        &self,
        identity: &SessionIdentity,
        home: &SessionHome,
    ) -> Result<(), DomainError> {
        self.store.record_new_home(identity, home)
    }
    fn discard_orphan(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.store.discard_orphan_home(identity)
    }
    fn list(&self) -> Result<HomeCatalogueSnapshot, DomainError> {
        self.list_joined(&mut JoinedWalk::default())
    }
}

impl FileSessionHomeCatalogue {
    /// The listing, and in `walk` what the store's walk would have listed
    /// and skipped — from the one pass (#2042).
    fn list_joined(&self, walk: &mut JoinedWalk) -> Result<HomeCatalogueSnapshot, DomainError> {
        // One list/publication at a time. The index seeds; stamps decide re-reads.
        let mut published = self.published.lock().map_err(error)?;
        let path = self.layout.home_catalogue_file();
        // Only a missing index is "absent"; a read failure is recovery.
        let disk = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => Some(format!("unreadable index: {e}").into_bytes()),
        };
        // Our own unchanged publication needs no decode and no re-seed.
        let ours = published
            .take()
            .filter(|(bytes, _)| Some(bytes) == disk.as_ref());
        let cached = match ours {
            Some((_, index)) => Some(Ok(index)),
            None => {
                let cached = disk.as_ref().map(|bytes| Catalogue::decode(bytes));
                match &cached {
                    Some(Ok(index)) => self.seed_projection(index)?,
                    // Rejections are never derived from the index: they stay.
                    _ => self.projection.lock().map_err(error)?.clear(),
                }
                cached
            }
        };
        let (authority, mut result) = self.scan(walk)?;
        match cached {
            Some(Ok(index)) if index == authority => {
                *published = disk.map(|bytes| (bytes, authority));
                return Ok(result);
            }
            // Superseded by newer authority (a routine autosave): refreshed silently.
            Some(Ok(_)) => {}
            // Unparseable or version-incompatible: recovery, reported.
            Some(Err(reason)) => {
                result.rebuilt = true;
                result
                    .diagnostics
                    .push(format!("home catalogue {reason}; rebuilt from authority"));
            }
            // Never existed (first use): built, not recovered — no report.
            None => {}
        }
        let bytes = serde_json::to_vec(&authority).map_err(error)?;
        match atomic_write(&path, &bytes, false) {
            Ok(()) => *published = Some((bytes, authority)),
            Err(e) => {
                *published = None;
                result
                    .diagnostics
                    .push(format!("home catalogue replacement unavailable: {e}"));
            }
        }
        Ok(result)
    }
}
