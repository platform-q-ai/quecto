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

#[path = "session_home_catalogue_index.rs"]
mod session_home_catalogue_index;
use session_home_catalogue_index::{Catalogue, HomeEntry, IndexEntry, IndexHome, SummaryEntry};
#[path = "session_home_catalogue_metadata.rs"]
mod session_home_catalogue_metadata;

#[derive(Clone)]
pub struct FileSessionHomeCatalogue {
    layout: FlatSessionLayout,
    store: std::sync::Arc<FileSessionStore>,
    published: std::sync::Arc<std::sync::Mutex<Published>>,
    projection: std::sync::Arc<std::sync::Mutex<BTreeMap<PathBuf, Projection>>>,
    transcript_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
/// One validated record, keyed in the projection by the identity's own
/// layout path: a projection can never yield another file's identity.
#[derive(Clone)]
struct Projection {
    identity: SessionIdentity,
    entry: IndexEntry,
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
            transcript_reads: Default::default(),
        }
    }
    #[cfg(feature = "test-support")]
    pub fn transcript_reads(&self) -> usize {
        self.transcript_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    fn scan(&self) -> Result<(Catalogue, HomeCatalogueSnapshot), DomainError> {
        let mut result = HomeCatalogueSnapshot {
            entries: Vec::new(),
            diagnostics: Vec::new(),
            rebuilt: false,
        };
        let mut records = BTreeMap::new();
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
                    self.scan_record(entry.path(), &mut records, &mut result)
                }
                Ok(_) => (),
                Err(e) => result
                    .diagnostics
                    .push(format!("session directory entry unavailable: {e}")),
            }
        }
        self.publish_stable_rows(&mut records, &mut result)?;
        result.entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok((Catalogue::new(records), result))
    }
    fn publish_stable_rows(
        &self,
        records: &mut Records,
        result: &mut HomeCatalogueSnapshot,
    ) -> Result<(), DomainError> {
        // A save/delete may have raced an earlier row while later rows were read.
        // Admit only rows whose validated authority still has the same signature.
        let mut projection = self.projection.lock().map_err(error)?;
        result.entries.retain(|(identity, home)| {
            let path = self.layout.session_file(identity);
            let valid = projection.get(&path).is_some_and(|cached| {
                stamp(&path).is_ok_and(|current| current == cached.entry.stamp)
            });
            // A published row and its index entry are written from one
            // observation; a mismatch is a programming error, never a
            // user-visible panic from inside `spawn_blocking`.
            debug_assert!(
                indexed_home_matches(records, identity, home),
                "published home observation must match its index entry"
            );
            if valid {
                true
            } else {
                records.remove(identity.runtime_key());
                projection.remove(&path);
                result
                    .diagnostics
                    .push("session changed before catalogue publication".into());
                false
            }
        });
        projection.retain(|path, _| path.exists());
        Ok(())
    }
    fn scan_record(
        &self,
        path: PathBuf,
        records: &mut Records,
        result: &mut HomeCatalogueSnapshot,
    ) {
        match self.projected_identity(&path) {
            Ok(identity) => {
                let home = self.observe_home(&path, &identity, records);
                debug_assert!(
                    indexed_home_matches(records, &identity, &home),
                    "published home observation must match its index entry"
                );
                if let SessionHomeScope::Unavailable(reason) = &home {
                    result
                        .diagnostics
                        .push(format!("home needs repair: {reason}"));
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
    /// The record's identity: the projection's when the file still carries
    /// the projected stamp, else one full read, strictly validated.
    fn projected_identity(&self, path: &PathBuf) -> Result<SessionIdentity, DomainError> {
        let before = stamp(path)?;
        if let Some(identity) = self.cached_if_current(path, &before)? {
            return Ok(identity);
        }
        self.transcript_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes = std::fs::read(path).map_err(error)?;
        let identity = identity_from_transcript(&bytes, path, &self.layout)?;
        super::session_store::session_store_catalogue::validate_catalogue_record(&bytes)?;
        debug_assert_eq!(self.layout.session_file(&identity), *path);
        let after = stamp(path)?;
        if before == after {
            self.projection.lock().map_err(error)?.insert(
                path.clone(),
                Projection {
                    identity: identity.clone(),
                    entry: IndexEntry {
                        stamp: after,
                        home: None,
                        summary: None,
                    },
                },
            );
            Ok(identity)
        } else {
            Err(error("session changed during catalogue validation"))
        }
    }
    fn cached_if_current(
        &self,
        path: &std::path::Path,
        before: &[u64],
    ) -> Result<Option<SessionIdentity>, DomainError> {
        Ok(self
            .projection
            .lock()
            .map_err(error)?
            .get(path)
            .filter(|cached| cached.entry.stamp == before)
            .map(|cached| cached.identity.clone()))
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
        // The store's walk validated its summary at this very stamp, or the
        // entry keeps the one it was seeded with (same stamp, same record).
        if let Some(summary) = self.store.summary_at(path, &cached.entry.stamp) {
            cached.entry.summary = Some(SummaryEntry {
                title: summary.title,
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
            let identity = SessionIdentity::from_persisted_key(key.as_str());
            if identity.persisted_key().is_some() && entry.stamp.len() == STAMP_LEN {
                projection.insert(
                    self.layout.session_file(&identity),
                    Projection {
                        identity,
                        entry: entry.clone(),
                    },
                );
            }
        }
        Ok(())
    }
}
/// The listing summaries the persisted index carries, keyed by record path
/// with the stamp each was validated at, for the store's summary walk to seed
/// from; an absent, unreadable or incompatible index seeds nothing. The walk
/// re-stamps every file, so a stale entry is read again, never trusted.
pub(super) fn persisted_summaries(
    layout: &FlatSessionLayout,
) -> BTreeMap<PathBuf, (Vec<u64>, crate::domain::session::SessionSummary)> {
    let Ok(bytes) = std::fs::read(layout.home_catalogue_file()) else {
        return BTreeMap::new();
    };
    let Ok(index) = Catalogue::decode(&bytes) else {
        return BTreeMap::new();
    };
    index
        .records
        .into_iter()
        .filter_map(|(key, entry)| {
            let summary = entry.summary?;
            let identity = SessionIdentity::from_persisted_key(key.as_str());
            if identity.persisted_key().is_none() || entry.stamp.len() != STAMP_LEN {
                return None;
            }
            let path = layout.session_file(&identity);
            let summary = crate::domain::session::SessionSummary {
                key,
                identity,
                title: summary.title,
                message_count: summary.message_count,
                updated_unix_secs: Some(entry.stamp[4]),
            };
            Some((path, (entry.stamp, summary)))
        })
        .collect()
}
fn identity_from_transcript(
    bytes: &[u8],
    path: &std::path::Path,
    layout: &FlatSessionLayout,
) -> Result<SessionIdentity, DomainError> {
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => first_jsonl_value(bytes)?,
    };
    let key = value
        .get("key")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error("record has no key"))?;
    let identity = SessionIdentity::from_persisted_key(key);
    if layout.session_file(&identity) == path && identity.persisted_key().is_some() {
        Ok(identity)
    } else {
        Err(error("record identity does not match layout"))
    }
}
fn first_jsonl_value(bytes: &[u8]) -> Result<serde_json::Value, DomainError> {
    // Validate every JSONL record: an in-flight partial append must never
    // publish a row based only on its intact snapshot header.
    let mut values = serde_json::Deserializer::from_slice(bytes).into_iter::<serde_json::Value>();
    let first = values
        .next()
        .ok_or_else(|| error("empty session"))?
        .map_err(error)?;
    for value in values {
        value.map_err(error)?;
    }
    Ok(first)
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
                    _ => self.projection.lock().map_err(error)?.clear(),
                }
                cached
            }
        };
        let (authority, mut result) = self.scan()?;
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
