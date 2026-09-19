//! Negative entries of the catalogue's projection (R1-H1): a record the
//! strict validation rejected, remembered by the stamp it was rejected at, so
//! an unchanged bad record costs one `stat` per query — like a good one — and
//! still yields its one file-named diagnostic. Persisted in the index beside
//! the valid entries (keyed by record file name), with what the store's walk
//! made of the same version, so a new process re-reads neither.
//!
//! Trust: reused only while the file carries the recorded stamp. A doctored
//! entry can at most hide a record from the *listing*, and names it in a
//! diagnostic while it does; exact-key resume never reads this index.
use super::super::session_layout::FlatSessionLayout;
use super::super::session_store::session_store_home::error;
use super::{STAMP_LEN, stamp};
use crate::domain::{error::DomainError, session_identity::SessionIdentity};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, path::PathBuf};

/// What the index carries for one rejected record version.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub(in crate::infrastructure::persistence) struct RejectedEntry {
    pub(in crate::infrastructure::persistence) stamp: Vec<u64>,
    pub(in crate::infrastructure::persistence) reason: String,
    /// The store walk's summary of this version (a crash-tolerant header
    /// parse can list what the strict catalogue rejects): key, title, count.
    #[serde(default)]
    pub(in crate::infrastructure::persistence) listed: Option<(String, String, usize)>,
    /// The store's walk could not summarise this version either.
    #[serde(default)]
    pub(in crate::infrastructure::persistence) unlisted: bool,
}

pub(in crate::infrastructure::persistence) type RejectedIndex = BTreeMap<String, RejectedEntry>;

#[derive(Default)]
pub(super) struct Rejections(BTreeMap<PathBuf, (Vec<u64>, String)>);

impl Rejections {
    /// The rejection of `path` at exactly `stamp`, as the error it was.
    pub(super) fn at(&self, path: &Path, stamp: &[u64]) -> Option<DomainError> {
        let (rejected_at, reason) = self.0.get(path)?;
        (rejected_at == stamp).then(|| DomainError::Session(reason.clone()))
    }
    pub(super) fn record(&mut self, path: &Path, stamp: Vec<u64>, error: &DomainError) {
        let reason = match error {
            DomainError::Session(reason) => reason.clone(),
            other => other.to_string(),
        };
        self.0.insert(path.to_path_buf(), (stamp, reason));
    }
    pub(super) fn retain_existing(&mut self) {
        self.0.retain(|path, _| path.exists());
    }
    /// Only well-formed entries naming a record file of this layout seed.
    pub(super) fn seed(&mut self, layout: &FlatSessionLayout, index: &RejectedIndex) {
        self.0 = index
            .iter()
            .filter_map(|(name, entry)| {
                let path = record_path(layout, name)?;
                (entry.stamp.len() == STAMP_LEN)
                    .then(|| (path, (entry.stamp.clone(), entry.reason.clone())))
            })
            .collect();
    }
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
    /// The cached rejection of `path`, for the index entry of this scan.
    pub(super) fn entry(&self, path: &Path) -> Option<(String, Vec<u64>, String)> {
        let (stamp, reason) = self.0.get(path)?;
        let name = path.file_name()?.to_str()?.to_string();
        Some((name, stamp.clone(), reason.clone()))
    }
}

/// A bare record file name of this layout — never a path out of it.
pub(in crate::infrastructure::persistence) fn record_path(
    layout: &FlatSessionLayout,
    name: &str,
) -> Option<PathBuf> {
    let path = layout.sessions_dir().join(name);
    let bare = Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name);
    (bare && FlatSessionLayout::is_session_record(&path)).then_some(path)
}

/// One full read, strictly validated: the identity the record names, which
/// must be the one its file name encodes.
pub(super) fn read_validated(
    path: &Path,
    layout: &FlatSessionLayout,
) -> Result<SessionIdentity, DomainError> {
    let bytes = std::fs::read(path).map_err(error)?;
    let identity = identity_from_transcript(&bytes, path, layout)?;
    super::super::session_store::session_store_catalogue::validate_catalogue_record(&bytes)?;
    debug_assert_eq!(layout.session_file(&identity), *path);
    Ok(identity)
}

/// A validation outcome is cacheable only when the file was stable around it.
pub(super) fn stable(path: &Path, before: &[u64]) -> Result<Vec<u64>, DomainError> {
    let after = stamp(path)?;
    if before == after {
        Ok(after)
    } else {
        Err(error("session changed during catalogue validation"))
    }
}

fn identity_from_transcript(
    bytes: &[u8],
    path: &Path,
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
    let identity = super::persisted_identity(key);
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
