//! Private metadata and admitted-content seam; interpretation stays in its existing owners.
use super::*;
type Metadata = BTreeMap<String, Found>;
type Content = BTreeMap<String, Vec<u8>>;
pub(crate) trait Reader {
    fn metadata(&self, ids: &[&str]) -> Result<Metadata, BoardStoreError>;
    fn listing(&self, tree: &str, cap: usize) -> Result<Vec<Entry>, Unread>;
    fn admitted(&self, wanted: &[(&str, usize)]) -> Result<Content, BoardStoreError>;
    /// Observational names from the sole interpreter's already-admitted files.
    /// Adapters must not infer admission by searching other object aliases.
    #[cfg(feature = "test-support")]
    fn witness_artifacts(&self, _paths: &[(String, String)]) {}
}
impl Reader for Git {
    fn metadata(&self, ids: &[&str]) -> Result<Metadata, BoardStoreError> {
        super::super::cat_file::objects(self, ids.iter().copied())
    }
    fn listing(&self, tree: &str, cap: usize) -> Result<Vec<Entry>, Unread> {
        let listing = self
            .ok_up_to(&["ls-tree", "-z", tree], None, cap)
            .map_err(unavailable)?;
        Ok(listing
            .split(|byte| *byte == 0)
            .filter(|row| !row.is_empty())
            .map(entry)
            .collect::<Result<_, _>>()?)
    }
    fn admitted(&self, wanted: &[(&str, usize)]) -> Result<Content, BoardStoreError> {
        super::super::cat_file::contents(self, wanted)
    }
}

pub(super) fn objects<'a>(
    reader: &impl Reader,
    ids: impl Iterator<Item = &'a str>,
) -> Result<Metadata, BoardStoreError> {
    let ids: BTreeSet<_> = ids.collect();
    let found = reader.metadata(&ids.iter().copied().collect::<Vec<_>>())?;
    match found.keys().map(String::as_str).eq(ids) {
        true => Ok(found),
        false => Err(BoardStoreError::Unavailable(
            "metadata identity mismatch".into(),
        )),
    }
}

pub(super) fn contents(
    reader: &impl Reader,
    wanted: &[(&str, usize)],
) -> Result<Content, BoardStoreError> {
    let found = match wanted.is_empty() {
        true => Content::new(),
        false => reader.admitted(wanted)?,
    };
    let ids: BTreeSet<_> = wanted.iter().map(|(id, _)| *id).collect();
    let valid = found.keys().map(String::as_str).eq(ids)
        && wanted
            .iter()
            .all(|(id, size)| found.get(*id).is_some_and(|bytes| bytes.len() == *size));
    match valid {
        true => Ok(found),
        false => Err(BoardStoreError::Unavailable(
            "admitted content identity or size mismatch".into(),
        )),
    }
}

#[cfg(test)]
#[path = "board_reader_tests.rs"]
mod tests;
