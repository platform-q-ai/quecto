//! Negative entries of the catalogue's projection (R1-H1): a record the
//! strict validation rejected, remembered by the stamp it was rejected at, so
//! an unchanged bad record costs one `stat` per query — like a good one — and
//! still yields its one file-named diagnostic on every answer.
//!
//! Only a CONTENT verdict is remembered (R2-H1): the bytes were read in full
//! and failed validation. An I/O failure is never recorded. And only in
//! memory, per process (R2-H2): a rejection is never written to the derived
//! index nor seeded from it, so the index can never hide a session and mixed
//! harness versions cannot hand each other a verdict. The cost is one read of
//! each corrupt record per process.
use super::super::session_layout::FlatSessionLayout;
use super::super::session_store::session_store_home::error;
use super::stamp;
use crate::domain::{error::DomainError, session_identity::SessionIdentity};
use serde::{Deserializer, de::IgnoredAny};
use std::{collections::BTreeMap, path::Path, path::PathBuf};

#[path = "session_home_catalogue_skipped.rs"]
mod session_home_catalogue_skipped;
pub(super) use session_home_catalogue_skipped::name_skipped;

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
    /// Memory only: an entry the scan did not see could never be looked up.
    pub(super) fn retain_seen(&mut self, seen: &std::collections::BTreeSet<PathBuf>) {
        self.0.retain(|path, _| seen.contains(path));
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

/// The index's legacy `rejected` key: ANY value under it, `null` too (R3-H6),
/// is "present", so the index is republished without it.
pub(super) fn legacy_key<'de, D: Deserializer<'de>>(d: D) -> Result<Option<IgnoredAny>, D::Error> {
    serde::Deserialize::deserialize(d).map(Some)
}

/// The strict validation of a record's bytes, read in full: the identity the
/// record names, which must be the one its file name encodes.
pub(super) fn validated(
    bytes: &[u8],
    path: &Path,
    layout: &FlatSessionLayout,
) -> Result<SessionIdentity, DomainError> {
    let identity = identity_from_transcript(bytes, path, layout)?;
    super::super::session_store::session_store_catalogue::validate_catalogue_record(bytes)?;
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

#[cfg(test)]
#[path = "session_home_catalogue_rejections_tests.rs"]
mod tests;
