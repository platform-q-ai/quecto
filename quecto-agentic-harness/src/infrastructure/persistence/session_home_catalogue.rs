//! Discardable discovery index. Every query validates against authority; exact
//! reads never depend on the index. Atomic replacement failures are diagnostics.
use super::{
    session_layout::FlatSessionLayout,
    session_store::{
        FileSessionStore,
        session_store_home::{atomic_write, error},
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

pub struct FileSessionHomeCatalogue {
    layout: FlatSessionLayout,
    store: std::sync::Arc<FileSessionStore>,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Catalogue {
    version: u32,
    /// Strong content digests detect freshness without copying transcripts.
    records: BTreeMap<String, Vec<u8>>,
}
impl FileSessionHomeCatalogue {
    pub fn with_store(layout: FlatSessionLayout, store: std::sync::Arc<FileSessionStore>) -> Self {
        Self { layout, store }
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
        result.entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok((
            Catalogue {
                version: 1,
                records,
            },
            result,
        ))
    }
    fn scan_record(
        &self,
        path: PathBuf,
        records: &mut BTreeMap<String, Vec<u8>>,
        result: &mut HomeCatalogueSnapshot,
    ) {
        let parsed = (|| -> Result<SessionIdentity, DomainError> {
            let bytes = std::fs::read(&path).map_err(error)?;
            let value: serde_json::Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(_) => {
                    // Validate every JSONL record: an in-flight partial append must
                    // never publish a row based only on its intact snapshot header.
                    let mut values = serde_json::Deserializer::from_slice(&bytes)
                        .into_iter::<serde_json::Value>();
                    let first = values
                        .next()
                        .ok_or_else(|| error("empty session"))?
                        .map_err(error)?;
                    for value in values {
                        value.map_err(error)?;
                    }
                    first
                }
            };
            super::session_store::validate_catalogue_record(&bytes)?;
            let key = value
                .get("key")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| error("record has no key"))?;
            let identity = SessionIdentity::from_persisted_key(key);
            if self.layout.session_file(&identity) == path && identity.persisted_key().is_some() {
                use sha2::Digest;
                records.insert(
                    format!("record:{key}"),
                    sha2::Sha256::digest(&bytes).to_vec(),
                );
                Ok(identity)
            } else {
                Err(error("record identity does not match layout"))
            }
        })();
        match parsed {
            Ok(identity) => {
                match std::fs::read(self.layout.home_file(&identity)) {
                    Ok(bytes) => {
                        records.insert(format!("home:{}", identity.runtime_key()), bytes);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => result.diagnostics.push(format!("home unreadable: {e}")),
                }
                let home = self
                    .store
                    .read_home(&identity)
                    .unwrap_or_else(|e| SessionHomeScope::Unavailable(e.to_string()));
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
}
impl SessionHomeCatalogue for FileSessionHomeCatalogue {
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
        let (authority, mut result) = self.scan()?;
        let path = self.layout.home_catalogue_file();
        let cached = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Catalogue>(&bytes).ok());
        if cached.as_ref() == Some(&authority) {
            return Ok(result);
        }
        result.rebuilt = true;
        result
            .diagnostics
            .push("home catalogue absent, stale, or invalid; rebuilt from authority".into());
        let bytes = serde_json::to_vec(&authority).map_err(error)?;
        if let Err(e) = atomic_write(&path, &bytes, false) {
            result
                .diagnostics
                .push(format!("home catalogue replacement unavailable: {e}"));
        }
        Ok(result)
    }
}
