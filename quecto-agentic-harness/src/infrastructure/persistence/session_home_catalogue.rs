//! Discardable discovery index. Every query validates against authority; exact
//! reads never depend on the index. Atomic replacement failures are diagnostics.
use super::{
    session_layout::FlatSessionLayout,
    session_store::{
        FileSessionStore,
        session_store_home::{atomic_write, decode, error},
    },
};
use crate::{
    application::sessions::ports::session_home::{HomeCatalogueSnapshot, SessionHomeCatalogue},
    domain::{
        error::DomainError,
        session_home::{SessionHome, SessionHomeScope},
        session_identity::SessionIdentity,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone)]
pub struct FileSessionHomeCatalogue {
    layout: FlatSessionLayout,
    store: std::sync::Arc<FileSessionStore>,
    published: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    projection: std::sync::Arc<std::sync::Mutex<BTreeMap<PathBuf, Projection>>>,
    transcript_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
#[derive(Clone)]
struct Projection {
    stamp: Vec<u64>,
    identity: SessionIdentity,
    digest: Vec<u8>,
}

/// Include ctime and inode, not merely user-restorable mtime/length. Only regular
/// files qualify; changed/replaced authorities must be validated again.
pub(super) fn stamp(path: &std::path::Path) -> Result<Vec<u64>, DomainError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(error)?;
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

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Catalogue {
    version: u32,
    /// Strong content digests detect freshness without copying transcripts.
    records: BTreeMap<String, Vec<u8>>,
}
impl FileSessionHomeCatalogue {
    pub fn with_store(layout: FlatSessionLayout, store: std::sync::Arc<FileSessionStore>) -> Self {
        Self {
            layout,
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
                return Ok((
                    Catalogue {
                        version: 1,
                        records,
                    },
                    result,
                ));
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
        Ok((
            Catalogue {
                version: 1,
                records,
            },
            result,
        ))
    }
    fn publish_stable_rows(
        &self,
        records: &mut BTreeMap<String, Vec<u8>>,
        result: &mut HomeCatalogueSnapshot,
    ) -> Result<(), DomainError> {
        // A save/delete may have raced an earlier row while later rows were read.
        // Admit only rows whose validated authority still has the same signature.
        let mut projection = self.projection.lock().map_err(error)?;
        result.entries.retain(|(identity, home)| {
            let path = self.layout.session_file(identity);
            let valid = projection
                .get(&path)
                .is_some_and(|entry| stamp(&path).is_ok_and(|current| current == entry.stamp));
            assert!(
                fingerprinted_home_matches(records, identity, home),
                "published home observation must match the fingerprinted home bytes"
            );
            if valid {
                true
            } else {
                records.remove(&format!("record:{}", identity.runtime_key()));
                records.remove(&format!("home:{}", identity.runtime_key()));
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
        records: &mut BTreeMap<String, Vec<u8>>,
        result: &mut HomeCatalogueSnapshot,
    ) {
        match self.projected_identity(&path, records) {
            Ok(identity) => {
                let home = self.observe_home(&identity, records);
                assert!(
                    fingerprinted_home_matches(records, &identity, &home),
                    "published home observation must match the fingerprinted home bytes"
                );
                if let SessionHomeScope::Unavailable(reason) = &home {
                    result
                        .diagnostics
                        .push(format!("home needs repair: {reason}"));
                }
                result.entries.push((identity, home));
            }
            Err(e) => result
                .diagnostics
                .push(format!("session record unavailable: {e}")),
        }
    }
    fn projected_identity(
        &self,
        path: &PathBuf,
        records: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<SessionIdentity, DomainError> {
        let before = stamp(path)?;
        if let Some(cached) = self.cached_if_current(path, &before)? {
            records.insert(
                format!("record:{}", cached.identity.runtime_key()),
                cached.digest,
            );
            return Ok(cached.identity);
        }
        self.transcript_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes = std::fs::read(path).map_err(error)?;
        let identity = identity_from_transcript(&bytes, path, &self.layout)?;
        super::session_store::session_store_catalogue::validate_catalogue_record(&bytes)?;
        debug_assert_eq!(self.layout.session_file(&identity), *path);
        let after = stamp(path)?;
        if before == after {
            use sha2::Digest;
            let digest = sha2::Sha256::digest(&bytes).to_vec();
            self.projection.lock().map_err(error)?.insert(
                path.clone(),
                Projection {
                    stamp: after,
                    identity: identity.clone(),
                    digest: digest.clone(),
                },
            );
            records.insert(format!("record:{}", identity.runtime_key()), digest);
            Ok(identity)
        } else {
            Err(error("session changed during catalogue validation"))
        }
    }
    fn cached_if_current(
        &self,
        path: &std::path::Path,
        before: &[u64],
    ) -> Result<Option<Projection>, DomainError> {
        Ok(self
            .projection
            .lock()
            .map_err(error)?
            .get(path)
            .cloned()
            .filter(|entry| entry.stamp == before))
    }
    fn observe_home(
        &self,
        identity: &SessionIdentity,
        records: &mut BTreeMap<String, Vec<u8>>,
    ) -> SessionHomeScope {
        // Decode the exact observation fingerprinted in the index: a
        // concurrent atomic home replacement cannot mix two versions.
        match std::fs::read(self.layout.home_file(identity)) {
            Ok(bytes) => {
                let home = decode(&bytes);
                records.insert(format!("home:{}", identity.runtime_key()), bytes);
                home
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::symlink_metadata(self.layout.home_file(identity)) {
                    Err(absent) if absent.kind() == std::io::ErrorKind::NotFound => {
                        SessionHomeScope::LegacyUnscoped
                    }
                    _ => SessionHomeScope::Unavailable("home authority is inaccessible".into()),
                }
            }
            Err(e) => SessionHomeScope::Unavailable(format!("home authority unreadable: {e}")),
        }
    }
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
fn fingerprinted_home_matches(
    records: &BTreeMap<String, Vec<u8>>,
    identity: &SessionIdentity,
    home: &SessionHomeScope,
) -> bool {
    match records.get(&format!("home:{}", identity.runtime_key())) {
        Some(bytes) => &decode(bytes) == home,
        None => matches!(
            home,
            SessionHomeScope::LegacyUnscoped | SessionHomeScope::Unavailable(_)
        ),
    }
}
impl SessionHomeCatalogue for FileSessionHomeCatalogue {
    fn list_async(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HomeCatalogueSnapshot, DomainError>>
                + Send
                + '_,
        >,
    > {
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
    fn list(&self) -> Result<HomeCatalogueSnapshot, DomainError> {
        // Serialize list/publication within this adapter. Disk data never seeds
        // the trusted projection: restart and externally changed indexes rebuild.
        let mut published = self.published.lock().map_err(error)?;
        let path = self.layout.home_catalogue_file();
        let disk = std::fs::read(&path).ok();
        if published
            .as_ref()
            .is_some_and(|previous| Some(previous) == disk.as_ref())
        {
            // Only our unchanged publication permits incremental reuse.
        } else {
            self.projection.lock().map_err(error)?.clear();
        }
        let cached = disk
            .as_ref()
            .and_then(|bytes| serde_json::from_slice::<Catalogue>(bytes).ok());
        let (authority, mut result) = self.scan()?;
        if cached.as_ref() == Some(&authority) {
            *published = disk;
            return Ok(result);
        }
        result.rebuilt = true;
        result
            .diagnostics
            .push("home catalogue absent, stale, or invalid; rebuilt from authority".into());
        let bytes = serde_json::to_vec(&authority).map_err(error)?;
        match atomic_write(&path, &bytes, false) {
            Ok(()) => *published = Some(bytes),
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
